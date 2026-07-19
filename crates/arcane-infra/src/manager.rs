//! ArcaneManager (IN-01) — central coordinator.

use arcane_core::{
    clustering_model::{ClusterInfo, PlayerInfo, WorldStateView},
    types::{Vec2, Vec3},
    IClusteringModel, IServerPool, ServerHandle,
};
use arcane_pool::LocalPool;
use arcane_rules::RulesEngine;
use arcane_spatial::SpatialIndex;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[cfg(feature = "migration")]
use crate::ownership_migration::OwnershipFlip;

#[cfg(feature = "migration")]
use arcane_affinity::cold_pair::sweep_cold_pairs;
#[cfg(feature = "migration")]
use arcane_affinity::config::AffinityConfig;
#[cfg(feature = "migration")]
use arcane_affinity::feature_map::FeatureMap;
#[cfg(feature = "migration")]
use arcane_affinity::interaction_graph::{Colocation, InteractionGraph, InteractionKind};
#[cfg(feature = "migration")]
use arcane_affinity::partition::{
    GreedyGrowthPartitioner, IPartitioner, PartitionInput, WeightedEdge,
};
#[cfg(feature = "migration")]
use arcane_affinity::predictor::{HeuristicPredictor, InteractionPredictor, PairContext};
#[cfg(feature = "migration")]
use arcane_affinity::refinement::{refine, RefineConfig};

// Stubs for non-migration mode
#[cfg(not(feature = "migration"))]
type AffinityConfig = ();
#[cfg(not(feature = "migration"))]
type FeatureMap = ();

/// Central coordinator: assignments, topology, clustering model.
pub struct ArcaneManager {
    model: Arc<dyn IClusteringModel>,
    pool: Arc<dyn IServerPool>,
    spatial_index: SpatialIndex,
    /// Allocated nodes. active_count = allocated_servers.len().
    allocated_servers: Vec<ServerHandle>,
    /// Entity dynamic features for edge rule matching.
    #[cfg_attr(not(feature = "migration"), allow(dead_code))]
    features: HashMap<Uuid, FeatureMap>,
    /// Affinity configuration: tuning constants and edge rules.
    #[cfg_attr(not(feature = "migration"), allow(dead_code))]
    config: AffinityConfig,
    /// Physics-coupling edges between entity pairs (Joint / Collision / PhysicsImpulse), keyed
    /// by the canonical ordered pair. These carry a `Colocation` class into the partitioner:
    /// `Hard` (Joint) is uncuttable, `CutFree` (SharedDeterministic) is free to cut. This is the
    /// seam a physics backend (Rapier) feeds; without it the Manager only ever sees Soft social
    /// and proximity edges and could never honor a joint constraint (design: interaction-edge
    /// taxonomy).
    #[cfg(feature = "migration")]
    physics_edges: HashMap<(Uuid, Uuid), Colocation>,
    /// Persistent, decaying interaction graph recording interactions across cycles.
    /// Accumulates weight from proximity/physics/feature-rule signals and decays over time,
    /// so transient signals don't flap the partition but sustained interaction builds strong edges.
    #[cfg(feature = "migration")]
    interaction_graph: InteractionGraph,
    /// Track last-seen entity set for removing departed entities from the graph.
    #[cfg(feature = "migration")]
    last_seen_entities: std::collections::HashSet<Uuid>,
    /// Migration guardrails (feature-gated).
    #[cfg(feature = "migration")]
    migration_state: MigrationState,
    /// Registered cluster topology (bootstrap + warm spares). Partitioning counts
    /// these as available partitions even when they own zero entities; without
    /// this an everyone-on-one-cluster world can never spread (k would be 1).
    #[cfg(feature = "migration")]
    known_clusters: Vec<Uuid>,
}

/// Migration guardrails: cooldown, in-flight cap, and per-node CPU cap config.
#[cfg(feature = "migration")]
#[derive(Debug)]
struct MigrationState {
    /// Last migration tick per entity. Enforces cooldown between migrations.
    last_migrated: HashMap<Uuid, u64>,
    /// Cooldown ticks between re-migrations of the same entity.
    cooldown_ticks: u64,
    /// Number of migrations currently pending (in-flight).
    in_flight_count: usize,
    /// Maximum concurrent pending migrations.
    max_in_flight: usize,
    /// Current tick counter for cooldown tracking.
    current_tick: u64,
    /// Ownership-flip decisions made this cycle, awaiting drain by the caller.
    /// The Manager decides but never publishes (design §3: it never talks to clusters
    /// directly). The caller drains these via `ArcaneManager::take_pending_flips` and
    /// actuates them (the Router's job in the target architecture).
    pending_flips: Vec<OwnershipFlip>,
}

/// Build partition-based migration decisions from the world view.
///
/// This function:
/// 1. Builds a weighted edge list from the persistent interaction graph
/// 2. Blends prediction into soft edge weights (cut_cost * (1 + config.prediction_gain * p))
/// 3. Runs the global GreedyGrowthPartitioner
/// 4. Runs refinement
/// 5. Maps partition indices to actual cluster ids deterministically
/// 6. Returns the desired assignments
#[cfg(feature = "migration")]
fn build_partition_decisions(
    view: &WorldStateView,
    current_assignments: &HashMap<Uuid, Uuid>,
    physics_edges: &HashMap<(Uuid, Uuid), Colocation>,
    interaction_graph: &InteractionGraph,
    config: &AffinityConfig,
    known_clusters: &[Uuid],
) -> HashMap<Uuid, Uuid> {
    // Collect all entity ids from the view
    let entities: Vec<Uuid> = view.players.iter().map(|p| p.player_id).collect();

    if entities.is_empty() {
        return HashMap::new();
    }

    // Build player position/velocity map for predictor
    let mut player_map: HashMap<Uuid, &PlayerInfo> = HashMap::new();
    for player in &view.players {
        player_map.insert(player.player_id, player);
    }

    // Instantiate predictor for edge weighting
    let predictor = HeuristicPredictor::default();

    // Build weighted edge list from the interaction graph.
    let mut edges: Vec<WeightedEdge> = Vec::new();
    let present: std::collections::HashSet<Uuid> = entities.iter().copied().collect();

    // Iterate all pairs from the graph with non-zero weight.
    for (a, b, weight) in interaction_graph.pairs() {
        // Skip pairs where one or both entities are not in the current view.
        if !present.contains(&a) || !present.contains(&b) {
            continue;
        }

        // Determine colocation class:
        // - Hard if the pair has any uncuttable (Joint) edge
        // - CutFree if all edges are CutFree
        // - Soft otherwise (with weight = cut_cost for Soft aggregate)
        let colocation = if interaction_graph.is_uncuttable(a, b) {
            Colocation::Hard
        } else {
            let cut_cost = interaction_graph.cut_cost(a, b);
            if cut_cost == 0.0 {
                Colocation::CutFree
            } else {
                Colocation::Soft
            }
        };

        // For Soft edges, blend prediction into the weight.
        let final_weight = if colocation == Colocation::Soft {
            let base_weight = interaction_graph.cut_cost(a, b);

            // Compute predictive enhancement if both players are in view
            let predicted_p = if let (Some(player_a), Some(player_b)) =
                (player_map.get(&a), player_map.get(&b))
            {
                let closing_speed = {
                    let dx = player_b.position.x - player_a.position.x;
                    let dy = player_b.position.y - player_a.position.y;
                    let distance = (dx * dx + dy * dy).sqrt();
                    let rel_vel_x = player_b.velocity.x - player_a.velocity.x;
                    let rel_vel_y = player_b.velocity.y - player_a.velocity.y;
                    if distance > 1e-9 {
                        -(rel_vel_x * dx + rel_vel_y * dy) / distance
                    } else {
                        0.0
                    }
                };

                let distance = {
                    let dx = player_b.position.x - player_a.position.x;
                    let dy = player_b.position.y - player_a.position.y;
                    (dx * dx + dy * dy).sqrt()
                };

                // Prediction is already incorporated into graph weights via the screen+predict pipeline.
                // Use empty feature maps here since features don't apply to graph edge blending.
                let empty_features = FeatureMap::new();
                let ctx = PairContext {
                    distance,
                    closing_speed,
                    horizon_secs: 5.0,
                    history_weight: base_weight,
                    features_a: &empty_features,
                    features_b: &empty_features,
                };
                predictor.predict(&ctx)
            } else {
                0.0
            };

            // Prediction-amplified weight: history-anchored, prediction-amplified
            base_weight * (1.0 + config.prediction_gain * predicted_p)
        } else {
            weight
        };

        edges.push(WeightedEdge {
            a,
            b,
            weight: final_weight,
            colocation,
        });
    }

    // Inject physics-coupling edges on top (current behavior) so a just-registered joint
    // constrains the very next cycle even before its graph weight exists.
    // For pairs where BOTH entities are currently in the view, these carry their co-location class
    // straight into the partitioner, so a joint constraint forces co-location and is never cut.
    if !physics_edges.is_empty() {
        for (&(a, b), &colocation) in physics_edges {
            if present.contains(&a) && present.contains(&b) {
                edges.push(WeightedEdge {
                    a,
                    b,
                    // Weight matters only for Soft edges; Hard/CutFree ignore it. Use a nominal
                    // positive weight so a Soft physics edge still contributes to the cut.
                    weight: 1.0,
                    colocation,
                });
            }
        }
    }

    // If no edges (no interactions), preserve current assignments (no reason to migrate).
    if edges.is_empty() {
        return current_assignments.clone();
    }

    // Number of partitions = number of KNOWN clusters (registered topology, including
    // empty warm spares), not merely clusters that currently own entities. With the
    // old "distinct current clusters" rule, a world where everyone starts on one
    // cluster yields k=1 forever — capacity can never force a spread because no
    // second partition exists to spread INTO. Warm spares must count.
    let num_partitions = {
        let mut clusters: Vec<Uuid> = current_assignments.values().copied().collect();
        clusters.extend_from_slice(known_clusters);
        clusters.sort();
        clusters.dedup();
        std::cmp::max(1, clusters.len())
    };

    // Capacity = ceil(n/k) * capacity_factor
    let base_capacity = entities.len().div_ceil(num_partitions);
    let capacity = std::cmp::max(1, (base_capacity as f64 * config.capacity_factor) as usize);

    // Build partition input
    let input = PartitionInput {
        entities: entities.clone(),
        edges,
        num_partitions,
        capacity,
    };

    // Run partitioner
    let partitioner = GreedyGrowthPartitioner::new();
    let partition = partitioner.partition(&input);

    // Run refinement
    let refined_partition = refine(
        &partition,
        &input.edges,
        num_partitions,
        &RefineConfig::default(),
    );

    // Map partition indices to cluster ids deterministically and INJECTIVELY:
    // two partitions must never map to the same cluster (the old plurality-only
    // rule collapsed all partitions onto the crowded cluster, so migrations to a
    // warm spare could never be emitted). Greedy assignment: process partitions
    // by decreasing size; each takes its plurality cluster if still free, else
    // the free known cluster with the most of its members, else any free known
    // cluster (sorted for determinism).
    let all_clusters: Vec<Uuid> = {
        let mut cs: Vec<Uuid> = current_assignments.values().copied().collect();
        cs.extend_from_slice(known_clusters);
        cs.sort();
        cs.dedup();
        cs
    };
    let mut free_clusters: std::collections::BTreeSet<Uuid> =
        all_clusters.iter().copied().collect();
    let mut order: Vec<usize> = (0..num_partitions).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(refined_partition.members(i).len()));

    let mut partition_to_cluster_id: HashMap<usize, Uuid> = HashMap::new();
    for part_idx in order {
        let members = refined_partition.members(part_idx);
        // Rank this partition's preference over FREE clusters by member plurality,
        // tie-break lowest Uuid (deterministic).
        let mut counts: HashMap<Uuid, usize> = HashMap::new();
        for member in &members {
            if let Some(&c) = current_assignments.get(member) {
                if free_clusters.contains(&c) {
                    *counts.entry(c).or_insert(0) += 1;
                }
            }
        }
        let chosen = counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(c, _)| c)
            .or_else(|| free_clusters.iter().next().copied());
        if let Some(c) = chosen {
            free_clusters.remove(&c);
            partition_to_cluster_id.insert(part_idx, c);
        }
    }

    // Produce final desired assignments from the partition
    let mut desired: HashMap<Uuid, Uuid> = HashMap::new();
    for entity in entities {
        if let Some(part_idx) = refined_partition.of(entity) {
            if let Some(&cluster_id) = partition_to_cluster_id.get(&part_idx) {
                desired.insert(entity, cluster_id);
            }
        }
    }

    desired
}

impl ArcaneManager {
    pub fn new(
        model: Arc<dyn IClusteringModel>,
        pool: Arc<dyn IServerPool>,
        spatial_index: SpatialIndex,
    ) -> Self {
        Self {
            model,
            pool,
            spatial_index,
            allocated_servers: Vec::new(),
            features: HashMap::new(),
            config: AffinityConfig::default(),
            #[cfg(feature = "migration")]
            physics_edges: HashMap::new(),
            #[cfg(feature = "migration")]
            interaction_graph: InteractionGraph::new(),
            #[cfg(feature = "migration")]
            last_seen_entities: std::collections::HashSet::new(),
            #[cfg(feature = "migration")]
            migration_state: MigrationState::new(),
            #[cfg(feature = "migration")]
            known_clusters: Vec::new(),
        }
    }

    /// Create with default LocalPool, RulesEngine, and fresh SpatialIndex (for tests / dev).
    pub fn with_defaults() -> Self {
        Self::new(
            Arc::new(RulesEngine::new()),
            Arc::new(LocalPool::default()),
            SpatialIndex::new(),
        )
    }

    /// Create with a named clustering model. Supported values: "rules" (default), "affinity".
    /// The "affinity" variant requires the `affinity-clustering` feature flag.
    pub fn with_model(model_type: &str) -> Self {
        let model: Arc<dyn IClusteringModel> = match model_type {
            #[cfg(feature = "affinity-clustering")]
            "affinity" => Arc::new(arcane_affinity::AffinityEngine::default()),
            _ => Arc::new(RulesEngine::new()),
        };
        Self::new(model, Arc::new(LocalPool::default()), SpatialIndex::new())
    }

    /// Feed entity position into the spatial index (e.g. from SpacetimeDB or test harness).
    pub fn update_entity(
        &mut self,
        entity_id: Uuid,
        cluster_id: Uuid,
        position: arcane_core::Vec3,
    ) {
        self.spatial_index
            .update_entity(entity_id, cluster_id, position);
    }

    /// Set observation radius used for neighbor discovery (delegates to SpatialIndex). Call before get_neighbors_for_cluster.
    pub fn set_observation_radius(&mut self, radius: f64) {
        self.spatial_index.set_observation_radius(radius);
    }

    /// Set the velocity for an entity (delegates to SpatialIndex).
    pub fn set_entity_velocity(&mut self, entity_id: Uuid, velocity: Vec3) {
        self.spatial_index
            .update_entity_velocity(entity_id, velocity);
    }

    /// Set a named feature value for an entity.
    pub fn set_entity_feature(&mut self, entity_id: Uuid, name: &str, value: f64) {
        #[cfg(feature = "migration")]
        {
            self.features
                .entry(entity_id)
                .or_default()
                .insert(name.to_string(), value);
        }
        #[cfg(not(feature = "migration"))]
        {
            let _ = (entity_id, name, value);
        }
    }

    /// Clear a named feature for an entity.
    pub fn clear_entity_feature(&mut self, entity_id: Uuid, name: &str) {
        #[cfg(feature = "migration")]
        {
            if let Some(fm) = self.features.get_mut(&entity_id) {
                fm.remove(name);
            }
        }
        #[cfg(not(feature = "migration"))]
        {
            let _ = (entity_id, name);
        }
    }

    /// Retrieve the FeatureMap for an entity, if any.
    pub fn entity_features(&self, entity_id: Uuid) -> Option<&FeatureMap> {
        #[cfg(feature = "migration")]
        {
            self.features.get(&entity_id)
        }
        #[cfg(not(feature = "migration"))]
        {
            let _ = entity_id;
            None
        }
    }

    /// Set the affinity configuration for tuning constants and edge rules.
    #[cfg(feature = "migration")]
    pub fn set_affinity_config(&mut self, config: AffinityConfig) {
        self.config = config;
    }

    /// No-op without the migration feature (AffinityConfig is `()` there).
    #[cfg(not(feature = "migration"))]
    pub fn set_affinity_config(&mut self, _config: AffinityConfig) {}

    /// Register the known cluster topology (bootstrap list + warm spares). The
    /// partitioner treats every known cluster as an available partition even when
    /// it currently owns nothing — this is what lets capacity pressure spread an
    /// everyone-on-one-cluster world onto empty spares.
    #[cfg(feature = "migration")]
    pub fn set_known_clusters(&mut self, clusters: Vec<Uuid>) {
        self.known_clusters = clusters;
    }

    /// Register (or clear) a physics-coupling edge between two entities, carrying its co-location
    /// class into the partitioner. `Colocation::Hard` (a Rapier joint) is uncuttable — the pair
    /// must never be split across clusters; `Colocation::CutFree` (a shared deterministic seed)
    /// costs nothing to cut; `Colocation::Soft` contributes weight. Pass `None` to remove the edge
    /// (e.g. a joint was destroyed). This is the seam the physics backend feeds; social/proximity
    /// edges are derived automatically from the view.
    ///
    /// The pair is stored canonically (min, max) so `set_physics_edge(a, b, ..)` and
    /// `set_physics_edge(b, a, ..)` refer to the same edge.
    #[cfg(feature = "migration")]
    pub fn set_physics_edge(&mut self, a: Uuid, b: Uuid, colocation: Option<Colocation>) {
        if a == b {
            return;
        }
        let key = if a <= b { (a, b) } else { (b, a) };
        match colocation {
            Some(c) => {
                self.physics_edges.insert(key, c);
            }
            None => {
                self.physics_edges.remove(&key);
            }
        }
    }

    /// Neighbor cluster IDs for a given cluster (from spatial index). Topology source for ReplicationChannelManager::set_neighbors.
    pub fn get_neighbors_for_cluster(&self, cluster_id: Uuid) -> Vec<Uuid> {
        self.spatial_index.get_neighbors(cluster_id)
    }

    /// Run one evaluation cycle: build view from spatial snapshot, run model, apply decisions.
    /// Without SpacetimeDB we allocate from pool when we have clusters (entities) and no servers yet.
    #[cfg(not(feature = "migration"))]
    pub fn run_evaluation_cycle(&mut self) -> Result<(), String> {
        let snapshot = self.spatial_index.snapshot_for_view();
        if snapshot.is_empty() {
            return Ok(());
        }

        // Build entity data for WorldStateView.players
        let entity_data = self.spatial_index.snapshot_entities();
        let mut cluster_player_ids: HashMap<uuid::Uuid, Vec<uuid::Uuid>> = HashMap::new();
        for &(entity_id, cluster_id, _) in &entity_data {
            cluster_player_ids
                .entry(cluster_id)
                .or_default()
                .push(entity_id);
        }

        let clusters: Vec<ClusterInfo> = snapshot
            .into_iter()
            .map(|g| ClusterInfo {
                cluster_id: g.cluster_id,
                server_host: "localhost".to_string(),
                player_ids: cluster_player_ids.remove(&g.cluster_id).unwrap_or_default(),
                player_count: g.entity_count,
                cpu_pct: 0.0,
                centroid: Vec2::new(g.centroid.x, g.centroid.z),
                spread_radius: g.spread_radius as f32,
                rpc_rate_out: 0.0,
            })
            .collect();

        let players: Vec<PlayerInfo> = entity_data
            .iter()
            .map(|&(entity_id, cluster_id, pos)| {
                let v = self
                    .spatial_index
                    .velocity_of(entity_id)
                    .unwrap_or(Vec3::new(0.0, 0.0, 0.0));
                PlayerInfo {
                    player_id: entity_id,
                    cluster_id,
                    position: Vec2::new(pos.x, pos.z),
                    velocity: Vec2::new(v.x, v.z),
                }
            })
            .collect();

        let view = WorldStateView {
            timestamp: 0.0,
            evaluation_budget_ms: 50,
            clusters,
            players,
        };
        let _decisions = self.model.evaluate(&view);
        // Minimal apply: if we have clusters in the world and no servers allocated, allocate one.
        if !self.allocated_servers.is_empty() {
            return Ok(());
        }
        match self.pool.allocate() {
            Ok(handle) => {
                self.allocated_servers.push(handle);
                Ok(())
            }
            Err(e) => Err(format!(
                "pool allocate failed: {} - {}",
                e.code as u32, e.detail
            )),
        }
    }

    /// Run one evaluation cycle with migration support (feature-gated).
    #[cfg(feature = "migration")]
    pub fn run_evaluation_cycle(&mut self) -> Result<(), String> {
        let snapshot = self.spatial_index.snapshot_for_view();
        if snapshot.is_empty() {
            return Ok(());
        }

        // Build entity data for WorldStateView.players
        let entity_data = self.spatial_index.snapshot_entities();
        let mut cluster_player_ids: HashMap<uuid::Uuid, Vec<uuid::Uuid>> = HashMap::new();
        for &(entity_id, cluster_id, _) in &entity_data {
            cluster_player_ids
                .entry(cluster_id)
                .or_default()
                .push(entity_id);
        }

        let clusters: Vec<ClusterInfo> = snapshot
            .into_iter()
            .map(|g| ClusterInfo {
                cluster_id: g.cluster_id,
                server_host: "localhost".to_string(),
                player_ids: cluster_player_ids.remove(&g.cluster_id).unwrap_or_default(),
                player_count: g.entity_count,
                cpu_pct: 0.0,
                centroid: Vec2::new(g.centroid.x, g.centroid.z),
                spread_radius: g.spread_radius as f32,
                rpc_rate_out: 0.0,
            })
            .collect();

        let players: Vec<PlayerInfo> = entity_data
            .iter()
            .map(|&(entity_id, cluster_id, pos)| {
                let v = self
                    .spatial_index
                    .velocity_of(entity_id)
                    .unwrap_or(Vec3::new(0.0, 0.0, 0.0));
                PlayerInfo {
                    player_id: entity_id,
                    cluster_id,
                    position: Vec2::new(pos.x, pos.z),
                    velocity: Vec2::new(v.x, v.z),
                }
            })
            .collect();

        let view = WorldStateView {
            timestamp: 0.0,
            evaluation_budget_ms: 50,
            clusters: clusters.clone(),
            players,
        };

        // Keep the existing evaluate() call for compatibility.
        let _decisions = self.model.evaluate(&view);

        self.migration_state.advance_tick();

        // Decay + GC the interaction graph using config values.
        self.interaction_graph.tick(
            self.config.decay_factor,
            self.config.gc_threshold,
            self.config.gc_interval,
        );

        // Record this cycle's signals into the graph.
        let players = &view.players;
        for i in 0..players.len() {
            for j in (i + 1)..players.len() {
                let a = &players[i];
                let b = &players[j];

                // Proximity pairs (using config radius and weight)
                let dx = a.position.x - b.position.x;
                let dy = a.position.y - b.position.y;
                let radius_sq = self.config.proximity_radius * self.config.proximity_radius;
                if dx * dx + dy * dy <= radius_sq {
                    self.interaction_graph.record_interaction(
                        a.player_id,
                        b.player_id,
                        self.config.proximity_weight,
                        InteractionKind::Proximity,
                    );
                }
            }
        }

        // Edge accumulation from edge rules: group entities by feature values.
        for edge_rule in &self.config.edge_rules {
            let mut feature_groups: HashMap<String, Vec<Uuid>> = HashMap::new();
            for player in players {
                if let Some(fm) = self.features.get(&player.player_id) {
                    if let Some(value) = fm.get(&edge_rule.feature) {
                        feature_groups
                            .entry(value.to_string())
                            .or_default()
                            .push(player.player_id);
                    }
                }
            }

            // Record pairwise edges within each group.
            for group in feature_groups.values() {
                for i in 0..group.len() {
                    for j in (i + 1)..group.len() {
                        self.interaction_graph.record_interaction(
                            group[i],
                            group[j],
                            edge_rule.weight,
                            InteractionKind::GameAction,
                        );
                    }
                }
            }
        }

        // Record physics-coupling edges (also kept in physics_edges for hard injection).
        for (&(a, b), &colocation) in &self.physics_edges {
            let kind = match colocation {
                Colocation::Hard => InteractionKind::Joint,
                Colocation::CutFree => InteractionKind::SharedDeterministic,
                Colocation::Soft => InteractionKind::Collision,
            };
            self.interaction_graph.record_interaction(a, b, 1.0, kind);
        }

        // Unified screen+predict pipeline for cold-pair promotion.
        // Screen pass: find candidate pairs from spatial + graph + feature proximity.
        let players_array: Vec<(Uuid, Vec2, Vec2)> = view
            .players
            .iter()
            .map(|p| (p.player_id, p.position, p.velocity))
            .collect();
        let features_array: Vec<(Uuid, FeatureMap)> = view
            .players
            .iter()
            .map(|p| {
                let fm = self
                    .features
                    .get(&p.player_id)
                    .cloned()
                    .unwrap_or_else(FeatureMap::new);
                (p.player_id, fm)
            })
            .collect();
        let edge_rules_array: Vec<(String, f64)> = self
            .config
            .edge_rules
            .iter()
            .map(|r| (r.feature.clone(), r.weight))
            .collect();

        let screen_radius = self.config.proximity_radius * self.config.screen_radius_factor;
        let candidates = arcane_affinity::cold_pair::screen_candidates(
            &players_array,
            &features_array,
            &self.interaction_graph,
            screen_radius,
            self.config.screen_min_closing_speed,
            &edge_rules_array,
        );

        // Predict pass: run predictor on candidates.
        if !candidates.is_empty() {
            let feature_lookup: HashMap<Uuid, FeatureMap> = features_array.into_iter().collect();
            let promotions = sweep_cold_pairs(
                &candidates,
                &HeuristicPredictor::default(),
                &feature_lookup,
                &arcane_affinity::cold_pair::SweepConfig {
                    horizon_secs: self.config.horizon_secs,
                    promote_threshold: 0.1,
                },
            );

            for promotion in promotions {
                // Promoted pairs write with scaled weight
                self.interaction_graph.record_interaction(
                    promotion.a,
                    promotion.b,
                    self.config.promotion_weight_scale * promotion.p,
                    InteractionKind::GameAction,
                );
            }
        }

        // Clean up departed entities from the graph.
        let current_entities: std::collections::HashSet<Uuid> =
            view.players.iter().map(|p| p.player_id).collect();
        for entity in self.last_seen_entities.iter() {
            if !current_entities.contains(entity) {
                self.interaction_graph.remove_entity(*entity);
            }
        }
        self.last_seen_entities = current_entities;

        // Build a map of current cluster assignment from the view for comparison.
        let mut current_assignments: HashMap<Uuid, Uuid> = HashMap::new();
        for (entity_id, cluster_id, _) in &entity_data {
            current_assignments.insert(*entity_id, *cluster_id);
        }

        // Use partition-based decision: build weighted edge list, partition, refine, and map to cluster ids.
        let resolved = build_partition_decisions(
            &view,
            &current_assignments,
            &self.physics_edges,
            &self.interaction_graph,
            &self.config,
            &self.known_clusters,
        );

        for (entity_id, desired_cluster) in resolved {
            if let Some(&current_cluster) = current_assignments.get(&entity_id) {
                if desired_cluster != current_cluster {
                    // Pinned entities never migrate (config.pin_feature, game-declared
                    // name; nonzero value = pinned). v1 stand-in for client handoff:
                    // a live client connection anchors its entity to the join cluster.
                    if let Some(ref pin_name) = self.config.pin_feature {
                        let pinned = self
                            .features
                            .get(&entity_id)
                            .and_then(|fm| fm.get(pin_name))
                            .is_some_and(|v| *v != 0.0);
                        if pinned {
                            continue;
                        }
                    }
                    // Decision is to migrate this entity.
                    if self.migration_state.can_migrate(entity_id) {
                        let flip = OwnershipFlip {
                            entity_id,
                            from_cluster: current_cluster,
                            to_cluster: desired_cluster,
                            effective_tick: self.migration_state.current_tick,
                        };
                        self.migration_state.record_migration(flip);
                        eprintln!(
                            "Migration initiated for entity {} from {} to {}",
                            entity_id, current_cluster, desired_cluster
                        );
                    } else {
                        let reason = if self.migration_state.in_flight_count
                            >= self.migration_state.max_in_flight
                        {
                            "in-flight cap reached"
                        } else {
                            "entity in cooldown"
                        };
                        self.migration_state.log_declined(entity_id, reason);
                    }
                }
            }
        }

        // Minimal apply: if we have clusters in the world and no servers allocated, allocate one.
        if !self.allocated_servers.is_empty() {
            return Ok(());
        }
        match self.pool.allocate() {
            Ok(handle) => {
                self.allocated_servers.push(handle);
                Ok(())
            }
            Err(e) => Err(format!(
                "pool allocate failed: {} - {}",
                e.code as u32, e.detail
            )),
        }
    }

    /// Current number of active clusters (for tests / metrics).
    pub fn active_cluster_count(&self) -> u32 {
        self.allocated_servers.len() as u32
    }

    /// Drain the ownership-flip decisions produced by `run_evaluation_cycle`.
    ///
    /// The Manager decides migrations and records them but never publishes to clusters
    /// itself (design §3: the Manager writes decisions where the Router reads them, and
    /// never talks to clusters directly). The caller (a node/router/test harness) drains
    /// the decisions here and actuates them — publishing each `OwnershipFlip` via
    /// `OwnershipFlipPublisher`. Draining acknowledges the in-flight decisions, so the
    /// in-flight guardrail counter is decremented per drained flip.
    #[cfg(feature = "migration")]
    pub fn take_pending_flips(&mut self) -> Vec<OwnershipFlip> {
        let flips = std::mem::take(&mut self.migration_state.pending_flips);
        for _ in 0..flips.len() {
            self.migration_state.complete_migration();
        }
        flips
    }

    /// Snapshot of cluster geometry from the spatial index (for visualization / debugging).
    pub fn snapshot_for_view(&self) -> Vec<arcane_core::ClusterGeometry> {
        self.spatial_index.snapshot_for_view()
    }

    /// Accessor for the interaction graph (feature-gated, used by ManagerRuntime).
    #[cfg(feature = "migration")]
    pub fn interaction_graph(&self) -> &InteractionGraph {
        &self.interaction_graph
    }

    /// Snapshot of entity positions and velocities (feature-gated, used by ManagerRuntime).
    /// Returns (entity_id, cluster_id, position, velocity) for all known entities.
    #[cfg(feature = "migration")]
    pub fn snapshot_positions(&self) -> Vec<(Uuid, Uuid, arcane_core::Vec3, arcane_core::Vec3)> {
        self.spatial_index
            .snapshot_entities()
            .into_iter()
            .map(|(entity_id, cluster_id, position)| {
                let velocity = self
                    .spatial_index
                    .velocity_of(entity_id)
                    .unwrap_or(arcane_core::Vec3::new(0.0, 0.0, 0.0));
                (entity_id, cluster_id, position, velocity)
            })
            .collect()
    }
}

#[cfg(feature = "migration")]
impl MigrationState {
    fn new() -> Self {
        Self {
            last_migrated: HashMap::new(),
            cooldown_ticks: 10,
            in_flight_count: 0,
            max_in_flight: 5,
            current_tick: 1,
            pending_flips: Vec::new(),
        }
    }

    fn advance_tick(&mut self) {
        self.current_tick += 1;
    }

    /// Check if an entity can be migrated (not in cooldown, and under in-flight cap).
    fn can_migrate(&self, entity_id: Uuid) -> bool {
        let cooldown_elapsed = if let Some(&last_tick) = self.last_migrated.get(&entity_id) {
            self.current_tick.saturating_sub(last_tick) >= self.cooldown_ticks
        } else {
            true // Never migrated before, so cooldown doesn't apply
        };
        let under_cap = self.in_flight_count < self.max_in_flight;
        cooldown_elapsed && under_cap
    }

    /// Mark an entity as migrated and record the ownership-flip decision for the caller
    /// to drain and actuate. In-flight count increments here; it decrements when the
    /// decision is drained via `take_pending_flips` (see `complete_migration`).
    fn record_migration(&mut self, flip: OwnershipFlip) {
        self.last_migrated.insert(flip.entity_id, self.current_tick);
        self.in_flight_count += 1;
        self.pending_flips.push(flip);
    }

    /// Decrement in-flight count when a recorded decision is drained/acknowledged.
    fn complete_migration(&mut self) {
        if self.in_flight_count > 0 {
            self.in_flight_count -= 1;
        }
    }

    /// Log a declined decision.
    fn log_declined(&self, entity_id: Uuid, reason: &str) {
        eprintln!(
            "Migration declined for entity {}: {} (in-flight: {}/{})",
            entity_id, reason, self.in_flight_count, self.max_in_flight
        );
    }
}

#[cfg(all(test, feature = "migration"))]
mod migration_tests {
    use super::*;

    /// Build a minimal flip for an entity (from/to clusters are placeholders for guardrail tests).
    fn mk_flip(entity_id: Uuid) -> OwnershipFlip {
        OwnershipFlip {
            entity_id,
            from_cluster: Uuid::from_u128(0xA),
            to_cluster: Uuid::from_u128(0xB),
            effective_tick: 1,
        }
    }

    #[test]
    fn migration_state_can_migrate_initially_true() {
        let state = MigrationState::new();
        let entity = Uuid::from_u128(1);
        assert!(state.can_migrate(entity));
    }

    #[test]
    fn migration_state_respects_cooldown() {
        let mut state = MigrationState::new();
        let entity = Uuid::from_u128(1);

        // Record a migration
        state.record_migration(mk_flip(entity));
        assert!(
            !state.can_migrate(entity),
            "entity should be in cooldown immediately"
        );

        // Advance ticks but not enough to clear cooldown
        for _ in 0..5 {
            state.advance_tick();
        }
        assert!(
            !state.can_migrate(entity),
            "entity should still be in cooldown after 5 ticks"
        );

        // Advance enough ticks to clear cooldown
        for _ in 0..6 {
            state.advance_tick();
        }
        assert!(
            state.can_migrate(entity),
            "entity should be available after cooldown expires"
        );
    }

    #[test]
    fn migration_state_respects_in_flight_cap() {
        let mut state = MigrationState::new();
        let cap = state.max_in_flight;

        // Fill the in-flight cap
        for i in 0..cap {
            let entity = Uuid::from_u128(i as u128 + 1);
            assert!(
                state.can_migrate(entity),
                "should migrate until cap is reached"
            );
            state.record_migration(mk_flip(entity));
        }

        // Next entity should be blocked by cap
        let next_entity = Uuid::from_u128((cap + 1) as u128);
        assert!(
            !state.can_migrate(next_entity),
            "should reject migration when in-flight cap is reached"
        );
    }

    #[test]
    fn migration_state_completes_migration() {
        let mut state = MigrationState::new();
        let entity = Uuid::from_u128(1);

        state.record_migration(mk_flip(entity));
        assert_eq!(state.in_flight_count, 1);

        state.complete_migration();
        assert_eq!(state.in_flight_count, 0);
    }

    #[test]
    fn record_migration_records_pending_flip() {
        let mut state = MigrationState::new();
        let entity = Uuid::from_u128(7);
        state.record_migration(mk_flip(entity));
        assert_eq!(state.pending_flips.len(), 1);
        assert_eq!(state.pending_flips[0].entity_id, entity);
        assert_eq!(state.in_flight_count, 1);
    }

    #[test]
    fn take_pending_flips_drains_and_decrements_in_flight() {
        let mut manager = ArcaneManager::with_defaults();
        // Record two decisions directly on the guardrail state.
        manager
            .migration_state
            .record_migration(mk_flip(Uuid::from_u128(1)));
        manager
            .migration_state
            .record_migration(mk_flip(Uuid::from_u128(2)));
        assert_eq!(manager.migration_state.in_flight_count, 2);

        let drained = manager.take_pending_flips();
        assert_eq!(drained.len(), 2, "both recorded flips are drained");
        assert_eq!(
            manager.migration_state.in_flight_count, 0,
            "draining acknowledges the in-flight decisions"
        );
        // Second drain is empty.
        assert!(manager.take_pending_flips().is_empty());
    }
}

#[cfg(test)]
mod view_enrichment_tests {
    use super::*;

    #[test]
    fn test_velocity_storage_and_retrieval() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);
        let cluster_id = Uuid::from_u128(100);
        let position = arcane_core::Vec3 {
            x: 10.0,
            y: 0.0,
            z: 20.0,
        };
        let velocity = Vec3 {
            x: 1.5,
            y: 0.0,
            z: 2.5,
        };

        // Set up entity
        manager.update_entity(entity_id, cluster_id, position);
        manager.set_entity_velocity(entity_id, velocity);

        // Verify velocity is stored
        assert_eq!(manager.spatial_index.velocity_of(entity_id), Some(velocity));
    }

    // Feature-map storage is migration-only (FeatureMap is a () stub otherwise).
    #[cfg(feature = "migration")]
    #[test]
    fn test_entity_feature_storage() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);

        // Set features
        manager.set_entity_feature(entity_id, "party", 200.0);
        manager.set_entity_feature(entity_id, "guild", 300.0);

        // Verify storage
        let features = manager.entity_features(entity_id);
        assert!(features.is_some());
        assert_eq!(features.unwrap().get("party"), Some(&200.0));
        assert_eq!(features.unwrap().get("guild"), Some(&300.0));
    }

    #[cfg(feature = "migration")]
    #[test]
    fn test_entity_feature_removal() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);

        // Set and then remove feature
        manager.set_entity_feature(entity_id, "party", 200.0);
        assert_eq!(
            manager
                .entity_features(entity_id)
                .and_then(|f| f.get("party")),
            Some(&200.0)
        );

        manager.clear_entity_feature(entity_id, "party");
        assert_eq!(
            manager
                .entity_features(entity_id)
                .and_then(|f| f.get("party")),
            None
        );
    }

    /// Pinned entities never migrate; the identical unpinned setup DOES migrate.
    /// Two co-moving pairs split across clusters force partition pressure; the
    /// only difference between runs is the pin feature — so if the pinned run
    /// also migrates, the guard is genuinely absent (un-fakeable by tuning).
    #[cfg(feature = "migration")]
    #[test]
    fn pinned_entities_never_migrate() {
        fn run(pin: bool) -> usize {
            let mut manager = ArcaneManager::with_model("affinity");
            let mut config = AffinityConfig {
                pin_feature: pin.then(|| "anchor".to_string()),
                ..Default::default()
            };
            config.edge_rules.push(arcane_affinity::config::EdgeRule {
                feature: "group".to_string(),
                weight: 50.0,
            });
            manager.set_affinity_config(config);

            let c1 = Uuid::from_u128(100);
            let c2 = Uuid::from_u128(200);
            manager.set_known_clusters(vec![c1, c2]);
            // Pair (1,2) co-located but SPLIT across clusters with a strong
            // feature edge: the partitioner must want to co-locate them.
            let e1 = Uuid::from_u128(1);
            let e2 = Uuid::from_u128(2);
            manager.update_entity(e1, c1, arcane_core::Vec3::new(0.0, 0.0, 0.0));
            manager.update_entity(e2, c2, arcane_core::Vec3::new(1.0, 0.0, 1.0));
            manager.set_entity_feature(e1, "group", 7.0);
            manager.set_entity_feature(e2, "group", 7.0);
            if pin {
                manager.set_entity_feature(e1, "anchor", 1.0);
                manager.set_entity_feature(e2, "anchor", 1.0);
            }

            let mut flips = 0;
            for _ in 0..20 {
                manager.run_evaluation_cycle().expect("cycle");
                flips += manager.take_pending_flips().len();
            }
            flips
        }

        let unpinned_flips = run(false);
        let pinned_flips = run(true);
        assert!(
            unpinned_flips > 0,
            "control run must migrate (else the test proves nothing)"
        );
        assert_eq!(
            pinned_flips, 0,
            "pinned entities migrated {pinned_flips} times"
        );
    }

    #[cfg(feature = "migration")]
    #[test]
    fn test_worldstateview_reflects_entity_features() {
        let mut manager = ArcaneManager::with_defaults();
        manager.set_observation_radius(100.0);

        let entity1_id = Uuid::from_u128(1);
        let entity2_id = Uuid::from_u128(2);
        let cluster1_id = Uuid::from_u128(100);
        let cluster2_id = Uuid::from_u128(101);

        let pos1 = arcane_core::Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let pos2 = arcane_core::Vec3 {
            x: 10.0,
            y: 0.0,
            z: 10.0,
        };
        let vel1 = Vec3 {
            x: 1.0,
            y: 0.0,
            z: 2.0,
        };
        let vel2 = Vec3 {
            x: -1.0,
            y: 0.0,
            z: -2.0,
        };

        // Set up two entities with features and velocities
        manager.update_entity(entity1_id, cluster1_id, pos1);
        manager.update_entity(entity2_id, cluster2_id, pos2);
        manager.set_entity_velocity(entity1_id, vel1);
        manager.set_entity_velocity(entity2_id, vel2);
        manager.set_entity_feature(entity1_id, "party", 500.0);
        manager.set_entity_feature(entity2_id, "party", 500.0);

        // Run evaluation cycle
        let result = manager.run_evaluation_cycle();
        assert!(result.is_ok());

        // Verify snapshot contains the entities
        let snapshot_entities = manager.spatial_index.snapshot_entities();
        assert_eq!(snapshot_entities.len(), 2);

        // Verify velocity is retrieved correctly (x/z mapping per spec)
        for (entity_id, _, _pos) in &snapshot_entities {
            if *entity_id == entity1_id {
                let retrieved_vel = manager.spatial_index.velocity_of(entity1_id);
                assert_eq!(retrieved_vel, Some(vel1));
            } else if *entity_id == entity2_id {
                let retrieved_vel = manager.spatial_index.velocity_of(entity2_id);
                assert_eq!(retrieved_vel, Some(vel2));
            }
        }

        // Verify features are accessible
        assert_eq!(
            manager
                .entity_features(entity1_id)
                .and_then(|f| f.get("party")),
            Some(&500.0)
        );
        assert_eq!(
            manager
                .entity_features(entity2_id)
                .and_then(|f| f.get("party")),
            Some(&500.0)
        );
    }

    #[test]
    fn test_velocity_removed_with_entity() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);
        let cluster_id = Uuid::from_u128(100);
        let position = arcane_core::Vec3 {
            x: 10.0,
            y: 0.0,
            z: 20.0,
        };
        let velocity = Vec3 {
            x: 1.5,
            y: 0.0,
            z: 2.5,
        };

        // Set up entity with velocity
        manager.update_entity(entity_id, cluster_id, position);
        manager.set_entity_velocity(entity_id, velocity);
        assert_eq!(manager.spatial_index.velocity_of(entity_id), Some(velocity));

        // Remove entity
        manager.spatial_index.remove_entity(entity_id, cluster_id);

        // Verify velocity is removed
        assert_eq!(manager.spatial_index.velocity_of(entity_id), None);
    }
}
