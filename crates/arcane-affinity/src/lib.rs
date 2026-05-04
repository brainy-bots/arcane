pub mod config;
pub mod hysteresis;
pub mod interaction_graph;
pub mod scorer;

use config::AffinityConfig;
use hysteresis::MigrationState;
use interaction_graph::InteractionGraph;
use scorer::score_entity;

use arcane_core::{
    clustering_model::{
        ClusterDecision, DecisionReason, DecisionType, ModelInfo, ValidationResult, WorldStateView,
    },
    types::Vec2,
    IClusteringModel,
};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

pub struct AffinityEngine {
    config: AffinityConfig,
    interaction_graph: Mutex<InteractionGraph>,
    migration_state: Mutex<MigrationState>,
    current_assignments: Mutex<HashMap<Uuid, Uuid>>,
}

impl AffinityEngine {
    pub fn new(config: AffinityConfig) -> Self {
        Self {
            config,
            interaction_graph: Mutex::new(InteractionGraph::new()),
            migration_state: Mutex::new(MigrationState::new()),
            current_assignments: Mutex::new(HashMap::new()),
        }
    }

    /// Inner computation: update state and return per-entity desired assignments.
    /// Handles all 6 phases before decision translation.
    fn compute_assignments_inner(&self, view: &WorldStateView) -> HashMap<Uuid, Uuid> {
        let mut graph = self.interaction_graph.lock().unwrap();
        let mut migration = self.migration_state.lock().unwrap();
        let mut assignments = self.current_assignments.lock().unwrap();

        // Phase 1a: decay interaction graph
        graph.tick(
            self.config.decay_factor,
            self.config.gc_threshold,
            self.config.gc_interval,
        );

        // Phase 1b: inject party/guild signals
        let players = &view.players;
        for i in 0..players.len() {
            for j in (i + 1)..players.len() {
                let a = &players[i];
                let b = &players[j];

                if let (Some(pa), Some(pb)) = (a.party_id, b.party_id) {
                    if pa == pb {
                        graph.record_interaction(
                            a.player_id,
                            b.player_id,
                            self.config.weight_party_member,
                        );
                    }
                }

                if let (Some(ga), Some(gb)) = (a.guild_id, b.guild_id) {
                    if ga == gb {
                        graph.record_interaction(
                            a.player_id,
                            b.player_id,
                            self.config.weight_guild_member,
                        );
                    }
                }
            }
        }

        // Phase 1c: inject proximity signals
        let r_sq = self.config.proximity_radius * self.config.proximity_radius;
        for i in 0..players.len() {
            for j in (i + 1)..players.len() {
                let a = &players[i];
                let b = &players[j];
                let dx = a.position.x - b.position.x;
                let dy = a.position.y - b.position.y;
                if dx * dx + dy * dy <= r_sq {
                    graph.record_interaction(
                        a.player_id,
                        b.player_id,
                        self.config.weight_proximity_per_tick,
                    );
                }
            }
        }

        // Phase 2: tick migration cooldowns
        migration.tick();

        // Phase 3: build cluster membership and centroids
        let mut cluster_members: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for cluster in &view.clusters {
            cluster_members
                .entry(cluster.cluster_id)
                .or_default()
                .extend(cluster.player_ids.iter().copied());
        }
        // Also incorporate assignments for entities not yet in cluster.player_ids
        for player in players {
            cluster_members
                .entry(player.cluster_id)
                .or_default()
                .push(player.player_id);
        }
        // Dedup
        for members in cluster_members.values_mut() {
            members.sort_unstable();
            members.dedup();
        }

        let cluster_centroids: HashMap<Uuid, Vec2> = view
            .clusters
            .iter()
            .map(|c| (c.cluster_id, c.centroid))
            .collect();

        let cluster_sizes: HashMap<Uuid, usize> = cluster_members
            .iter()
            .map(|(id, members)| (*id, members.len()))
            .collect();

        // Phase 4: score each entity and decide migrations.
        // Seed from cache first, then fill gaps from view.players (authoritative current
        // assignment). This ensures entities that score below the migration threshold are
        // already in new_assignments with their current cluster — Phase 5 must not override them.
        let mut new_assignments: HashMap<Uuid, Uuid> = assignments.clone();
        for player in players {
            new_assignments
                .entry(player.player_id)
                .or_insert(player.cluster_id);
        }

        for player in players {
            let current_cluster = assignments
                .get(&player.player_id)
                .copied()
                .unwrap_or(player.cluster_id);

            if migration.is_on_cooldown(player.player_id) {
                continue;
            }

            if cluster_centroids.is_empty() {
                continue;
            }

            let result = score_entity(
                player.player_id,
                player.position,
                current_cluster,
                &cluster_members,
                &cluster_centroids,
                &cluster_sizes,
                &graph,
                &self.config,
            );

            let improvement = result.best_score - result.current_score;
            if result.best_cluster != current_cluster
                && improvement > self.config.migration_threshold
            {
                new_assignments.insert(player.player_id, result.best_cluster);
                migration.record_migration(player.player_id, self.config.cooldown_ticks);
            }
        }

        // Phase 5: new entities with no history → spatial fallback
        for player in players {
            if let std::collections::hash_map::Entry::Vacant(e) =
                new_assignments.entry(player.player_id)
            {
                if let Some(cid) = nearest_cluster(player.position, &cluster_centroids) {
                    e.insert(cid);
                }
            }
        }

        // Phase 6: clean up removed entities
        let active: std::collections::HashSet<Uuid> = players.iter().map(|p| p.player_id).collect();
        for entity in assignments
            .keys()
            .filter(|e| !active.contains(*e))
            .copied()
            .collect::<Vec<_>>()
        {
            graph.remove_entity(entity);
            migration.remove_entity(entity);
        }
        new_assignments.retain(|e, _| active.contains(e));

        *assignments = new_assignments.clone();
        new_assignments
    }

    /// Log per-tick metrics via tracing.
    fn emit_metrics(&self, entity_assignments: &HashMap<Uuid, Uuid>, view: &WorldStateView) {
        let graph = self.interaction_graph.lock().unwrap();
        let migration = self.migration_state.lock().unwrap();

        let cluster_sizes: HashMap<Uuid, usize> = {
            let mut m: HashMap<Uuid, usize> = HashMap::new();
            for &cid in entity_assignments.values() {
                *m.entry(cid).or_insert(0) += 1;
            }
            m
        };
        let max_size = cluster_sizes.values().copied().max().unwrap_or(0);
        let min_size = cluster_sizes.values().copied().min().unwrap_or(0);

        tracing::debug!(
            interaction_pairs = graph.pair_count(),
            migrations_blocked_cooldown = migration.cooldown_count(),
            max_cluster_size = max_size,
            min_cluster_size = min_size,
            total_players = view.players.len(),
            "affinity_engine_tick"
        );
    }
}

impl Default for AffinityEngine {
    fn default() -> Self {
        Self::new(AffinityConfig::default())
    }
}

impl IClusteringModel for AffinityEngine {
    fn evaluate(&self, view: &WorldStateView) -> Vec<ClusterDecision> {
        let entity_assignments = self.compute_assignments_inner(view);
        self.emit_metrics(&entity_assignments, view);

        // Phase 7: translate per-entity assignments into merge/split decisions
        assignments_to_decisions(&entity_assignments, view, &self.config)
    }

    fn get_model_info(&self) -> ModelInfo {
        ModelInfo {
            model_type: "affinity_engine".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            trained_at: None,
            feature_count: Some(3), // party, guild, proximity
        }
    }

    fn validate_view(&self, view: &WorldStateView) -> ValidationResult {
        let mut warnings = Vec::new();
        let mut errors = Vec::new();

        if view.players.is_empty() {
            warnings.push(
                "WorldStateView.players is empty — AffinityEngine running in degraded mode \
                 (proximity/party/guild signals unavailable)"
                    .to_string(),
            );
        }

        // Check player cluster references exist
        let cluster_ids: std::collections::HashSet<Uuid> =
            view.clusters.iter().map(|c| c.cluster_id).collect();
        for player in &view.players {
            if !cluster_ids.contains(&player.cluster_id) {
                errors.push(format!(
                    "player {} references unknown cluster {}",
                    player.player_id, player.cluster_id
                ));
            }
        }

        ValidationResult {
            valid: errors.is_empty(),
            warnings,
            errors,
        }
    }

    fn compute_entity_assignments(&self, view: &WorldStateView) -> HashMap<Uuid, Uuid> {
        self.compute_assignments_inner(view)
    }
}

/// Phase 7: convert per-entity desired assignments to merge/split ClusterDecisions.
fn assignments_to_decisions(
    entity_assignments: &HashMap<Uuid, Uuid>,
    view: &WorldStateView,
    config: &AffinityConfig,
) -> Vec<ClusterDecision> {
    // Build current cluster membership from view
    let mut current_cluster: HashMap<Uuid, Uuid> = HashMap::new();
    for cluster in &view.clusters {
        for &pid in &cluster.player_ids {
            current_cluster.insert(pid, cluster.cluster_id);
        }
    }
    for player in &view.players {
        current_cluster
            .entry(player.player_id)
            .or_insert(player.cluster_id);
    }

    // Count how many entities want to move from cluster A to cluster B
    let mut migration_counts: HashMap<(Uuid, Uuid), u32> = HashMap::new();
    for (&entity, &desired) in entity_assignments {
        let current = match current_cluster.get(&entity) {
            Some(&c) => c,
            None => continue,
        };
        if current != desired {
            *migration_counts.entry((current, desired)).or_insert(0) += 1;
        }
    }

    let mut decisions = Vec::new();
    let mut handled_pairs: std::collections::HashSet<(Uuid, Uuid)> =
        std::collections::HashSet::new();

    for ((src, dst), count) in &migration_counts {
        if *count < config.merge_entity_threshold as u32 {
            continue;
        }
        // Normalize pair to avoid duplicate decisions
        let key = if src < dst {
            (*src, *dst)
        } else {
            (*dst, *src)
        };
        if handled_pairs.contains(&key) {
            continue;
        }
        handled_pairs.insert(key);

        decisions.push(ClusterDecision {
            decision_type: DecisionType::Merge,
            priority: 5,
            reason: DecisionReason {
                code: "HIGH_INTERACTION_RATE".to_string(),
                detail: format!(
                    "{} entities have higher affinity with cluster {} than their current cluster {}",
                    count, dst, src
                ),
            },
            confidence: 1.0,
            source_cluster_id: Some(*src),
            target_cluster_id: Some(*dst),
            cluster_id: None,
            split_group_a: None,
            split_group_b: None,
        });
    }

    decisions
}

/// Find the nearest cluster centroid. Returns None if no clusters exist.
fn nearest_cluster(pos: Vec2, centroids: &HashMap<Uuid, Vec2>) -> Option<Uuid> {
    centroids
        .iter()
        .min_by(|(id_a, ca), (id_b, cb)| {
            let da = {
                let dx = pos.x - ca.x;
                let dy = pos.y - ca.y;
                dx * dx + dy * dy
            };
            let db = {
                let dx = pos.x - cb.x;
                let dy = pos.y - cb.y;
                dx * dx + dy * dy
            };
            da.partial_cmp(&db)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| id_a.cmp(id_b))
        })
        .map(|(&id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_core::clustering_model::{ClusterInfo, PlayerInfo};

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn make_view(clusters: Vec<ClusterInfo>, players: Vec<PlayerInfo>) -> WorldStateView {
        WorldStateView {
            timestamp: 0.0,
            evaluation_budget_ms: 50,
            clusters,
            players,
        }
    }

    fn cluster(id: Uuid, player_ids: Vec<Uuid>, cx: f64, cy: f64) -> ClusterInfo {
        ClusterInfo {
            cluster_id: id,
            server_host: "localhost".to_string(),
            player_ids,
            player_count: 0,
            cpu_pct: 0.0,
            centroid: Vec2::new(cx, cy),
            spread_radius: 0.0,
            rpc_rate_out: 0.0,
        }
    }

    fn player(id: Uuid, cluster_id: Uuid, x: f64, y: f64) -> PlayerInfo {
        PlayerInfo {
            player_id: id,
            cluster_id,
            position: Vec2::new(x, y),
            velocity: Vec2::new(0.0, 0.0),
            guild_id: None,
            party_id: None,
        }
    }

    #[test]
    fn valid_assignments_for_all_entities() {
        let c1 = uuid(10);
        let c2 = uuid(11);
        let p1 = uuid(1);
        let p2 = uuid(2);
        let p3 = uuid(3);

        let view = make_view(
            vec![
                cluster(c1, vec![p1, p2], 0.0, 0.0),
                cluster(c2, vec![p3], 100.0, 0.0),
            ],
            vec![
                player(p1, c1, 0.0, 0.0),
                player(p2, c1, 5.0, 0.0),
                player(p3, c2, 100.0, 0.0),
            ],
        );

        let engine = AffinityEngine::default();
        let result = engine.compute_entity_assignments(&view);

        // Every entity must be assigned to an existing cluster
        let cluster_ids: std::collections::HashSet<Uuid> = [c1, c2].into_iter().collect();
        for assigned_cluster in result.values() {
            assert!(cluster_ids.contains(assigned_cluster));
        }
    }

    #[test]
    fn validate_view_warns_on_empty_players() {
        let engine = AffinityEngine::default();
        let view = make_view(vec![cluster(uuid(1), vec![], 0.0, 0.0)], vec![]);
        let result = engine.validate_view(&view);
        assert!(result.valid);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn validate_view_errors_on_unknown_cluster() {
        let engine = AffinityEngine::default();
        let view = make_view(
            vec![cluster(uuid(1), vec![], 0.0, 0.0)],
            vec![player(uuid(2), uuid(99), 0.0, 0.0)], // cluster 99 doesn't exist
        );
        let result = engine.validate_view(&view);
        assert!(!result.valid);
    }

    #[test]
    fn get_model_info_returns_affinity_type() {
        let engine = AffinityEngine::default();
        let info = engine.get_model_info();
        assert_eq!(info.model_type, "affinity_engine");
        assert!(info.trained_at.is_none());
    }

    #[test]
    fn drop_in_replacement_no_panic() {
        let c1 = uuid(10);
        let c2 = uuid(11);
        let view = make_view(
            vec![
                cluster(c1, vec![uuid(1), uuid(2)], 0.0, 0.0),
                cluster(c2, vec![uuid(3)], 50.0, 0.0),
            ],
            vec![
                player(uuid(1), c1, 0.0, 0.0),
                player(uuid(2), c1, 2.0, 0.0),
                player(uuid(3), c2, 50.0, 0.0),
            ],
        );

        let engine = AffinityEngine::default();
        let decisions = engine.evaluate(&view);
        // No panic, decisions is a valid vec (may be empty)
        let _ = decisions;
    }

    // ── Integration / behavioural tests (#76) ──────────────────────────────────

    /// A raid group of 20 entities with heavy mutual interactions stays on the same
    /// cluster even after moving into the other cluster's spatial territory.
    ///
    /// Setup: cluster 1 centroid at x=-200, cluster 2 centroid at x=+200.
    /// All 20 entities start in cluster 1 at x=-5 and interact heavily.
    /// After building history, we reposition them to x=+5 (closer to C2 spatially).
    ///
    /// Expected: interaction score with C1 (all 19 partners in C1) >> spatial score
    /// for C2, so improvement = score(C2) - score(C1) is negative → no migration.
    #[test]
    fn raid_group_stays_together_across_boundary() {
        let c1 = uuid(10);
        let c2 = uuid(11);

        // 20 group members
        let members: Vec<Uuid> = (1u8..=20).map(uuid).collect();

        let engine = AffinityEngine::new(AffinityConfig {
            weight_game_action: 2.0,
            spatial_weight: 0.2,
            migration_threshold: 3.0,
            ..AffinityConfig::default()
        });

        // Phase A: build interaction history — all members fight each other in C1.
        // C1 centroid at x=-200, C2 centroid at x=+200.
        // Members at x=-5: clearly C1 territory spatially.
        let view_build = make_view(
            vec![
                cluster(c1, members.clone(), -200.0, 0.0),
                cluster(c2, vec![], 200.0, 0.0),
            ],
            members
                .iter()
                .map(|&id| player(id, c1, -5.0, 0.0))
                .collect(),
        );

        // Run 10 ticks of interaction: record game_action between every pair
        // by evaluating the view (proximity signal fires every tick since all at x=-5
        // and proximity_radius=50 covers them all).
        for _ in 0..10 {
            engine.evaluate(&view_build);
        }

        // Phase B: reposition all members to x=+5 — spatially closer to C2.
        // But they're still assigned to C1, and interaction history is rich.
        let view_moved = make_view(
            vec![
                cluster(c1, members.clone(), -200.0, 0.0),
                cluster(c2, vec![], 200.0, 0.0),
            ],
            members.iter().map(|&id| player(id, c1, 5.0, 0.0)).collect(),
        );

        let assignments = engine.compute_entity_assignments(&view_moved);

        // All 20 members must be assigned to the same cluster (they stay together).
        let assigned_clusters: std::collections::HashSet<Uuid> =
            members.iter().map(|id| assignments[id]).collect();
        assert_eq!(
            assigned_clusters.len(),
            1,
            "raid group scattered across clusters: {:?}",
            assigned_clusters
        );
    }

    /// Two-part hysteresis test:
    ///
    /// Part 1 — threshold guard: entity has marginal interaction with C2 entities
    /// (weight < migration_threshold) so it stays in C1 despite spatial pull toward C2.
    ///
    /// Part 2 — cooldown guard: entity gets overwhelming interaction with C2 and
    /// migrates. Immediately after, cooldown prevents a re-migration back to C1.
    #[test]
    fn hysteresis_prevents_boundary_oscillation() {
        let c1 = uuid(10);
        let c2 = uuid(11);
        let entity = uuid(1);
        let c1_partner = uuid(2); // lives in C1
        let c2_partner = uuid(3); // lives in C2

        // Config: migration_threshold=3.0, cooldown_ticks=5 (short for test speed)
        let engine = AffinityEngine::new(AffinityConfig {
            spatial_weight: 0.0, // pure interaction — isolates the hysteresis logic
            migration_threshold: 3.0,
            cooldown_ticks: 5,
            ..AffinityConfig::default()
        });

        // ── Part 1: threshold guard ──────────────────────────────────────────
        // Entity is in C1. It has interaction weight 1.0 with a C2 entity.
        // improvement = score(C2) - score(C1) = 1.0 - 0.0 = 1.0 < 3.0 → no migration.

        // Seed a single interaction with the C2 partner (weight 1.0).
        {
            let mut graph = engine.interaction_graph.lock().unwrap();
            graph.record_interaction(entity, c2_partner, 1.0);
        }

        let view_threshold = make_view(
            vec![
                cluster(c1, vec![entity, c1_partner], 0.0, 0.0),
                cluster(c2, vec![c2_partner], 0.0, 0.0),
            ],
            vec![
                player(entity, c1, 0.0, 0.0),
                player(c1_partner, c1, 0.0, 0.0),
                player(c2_partner, c2, 0.0, 0.0),
            ],
        );

        let assignments = engine.compute_entity_assignments(&view_threshold);
        assert_eq!(
            assignments.get(&entity).copied().unwrap_or(c1),
            c1,
            "threshold guard failed: entity migrated with insufficient improvement"
        );

        // ── Part 2a: overwhelming interaction triggers migration ──────────────
        // Add strong interaction with C2 partner: total weight now >> migration_threshold.
        {
            let mut graph = engine.interaction_graph.lock().unwrap();
            graph.record_interaction(entity, c2_partner, 10.0);
        }
        // Also clear any existing assignment cache so entity starts fresh from C1.
        {
            let mut assignments_cache = engine.current_assignments.lock().unwrap();
            assignments_cache.insert(entity, c1);
        }

        let assignments2 = engine.compute_entity_assignments(&view_threshold);
        let entity_cluster_after_migration = assignments2.get(&entity).copied().unwrap_or(c1);
        assert_eq!(
            entity_cluster_after_migration, c2,
            "entity should have migrated to C2 with overwhelming interaction"
        );

        // ── Part 2b: cooldown prevents immediate re-migration ─────────────────
        // Entity is now in C2. Build a view that would spatially/interactionally
        // suggest C1 (add heavy interaction with C1 partner, clear C2 interaction).
        {
            let mut graph = engine.interaction_graph.lock().unwrap();
            // Replace weights: heavy C1 interaction, zero C2
            graph.remove_entity(c2_partner);
            graph.record_interaction(entity, c1_partner, 20.0);
        }

        let view_cooldown = make_view(
            vec![
                cluster(c1, vec![c1_partner], 0.0, 0.0),
                cluster(c2, vec![entity], 0.0, 0.0),
            ],
            vec![
                player(entity, c2, 0.0, 0.0),
                player(c1_partner, c1, 0.0, 0.0),
            ],
        );

        // Entity just migrated — must be on cooldown → stays in C2 despite C1 pull.
        let assignments3 = engine.compute_entity_assignments(&view_cooldown);
        assert_eq!(
            assignments3.get(&entity).copied().unwrap_or(c2),
            c2,
            "cooldown guard failed: entity re-migrated within cooldown window"
        );
    }

    /// An entity with no interaction history migrates to the spatially nearest cluster when the
    /// spatial improvement exceeds the migration threshold. migration_threshold=0.0 removes the
    /// gate so any positive spatial improvement is sufficient — this is the correct way to test
    /// that pure spatial scoring governs assignment when interaction history is absent.
    ///
    /// (Phase 5 spatial fallback only fires for entities with no cluster assignment at all;
    /// for assigned entities Phase 4 must produce sufficient improvement to trigger migration.)
    #[test]
    fn isolated_entity_uses_spatial_fallback() {
        let c1 = uuid(10);
        let c2 = uuid(11);
        let loner = uuid(1);
        let anchor = uuid(2); // gives c1 a member so centroid is populated

        let engine = AffinityEngine::new(AffinityConfig {
            migration_threshold: 0.0, // any positive spatial improvement triggers migration
            ..AffinityConfig::default()
        });

        // Loner has no interactions. C1 centroid at x=-200, C2 centroid at x=+5.
        // Loner is at x=0 — closer to C2. spatial improvement > 0 → migrates to C2.
        let view = make_view(
            vec![
                cluster(c1, vec![anchor], -200.0, 0.0),
                cluster(c2, vec![], 5.0, 0.0),
            ],
            vec![player(loner, c1, 0.0, 0.0), player(anchor, c1, -200.0, 0.0)],
        );

        let assignments = engine.compute_entity_assignments(&view);
        assert_eq!(
            assignments.get(&loner).copied().unwrap_or(c1),
            c2,
            "loner should migrate to spatially nearest cluster C2"
        );
    }
}
