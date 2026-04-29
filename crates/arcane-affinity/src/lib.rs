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

        // Phase 4: score each entity and decide migrations
        let mut new_assignments: HashMap<Uuid, Uuid> = assignments.clone();

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
            if !new_assignments.contains_key(&player.player_id) {
                let nearest = nearest_cluster(player.position, &cluster_centroids);
                if let Some(cid) = nearest {
                    new_assignments.insert(player.player_id, cid);
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
        .min_by(|(_, ca), (_, cb)| {
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
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
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
        for (_, assigned_cluster) in &result {
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
}
