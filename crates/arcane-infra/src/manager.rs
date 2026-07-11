//! ArcaneManager (IN-01) — central coordinator.

use arcane_core::{
    clustering_model::{ClusterInfo, PlayerInfo, WorldStateView},
    types::Vec2,
    IClusteringModel, IServerPool, ServerHandle,
};
use arcane_pool::LocalPool;
use arcane_rules::RulesEngine;
use arcane_spatial::SpatialIndex;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Central coordinator: assignments, topology, clustering model.
pub struct ArcaneManager {
    model: Arc<dyn IClusteringModel>,
    pool: Arc<dyn IServerPool>,
    spatial_index: SpatialIndex,
    /// Allocated nodes. active_count = allocated_servers.len().
    allocated_servers: Vec<ServerHandle>,
    /// Migration guardrails (feature-gated).
    #[cfg(feature = "migration")]
    migration_state: MigrationState,
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
            #[cfg(feature = "migration")]
            migration_state: MigrationState::new(),
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
            .map(|&(entity_id, cluster_id, pos)| PlayerInfo {
                player_id: entity_id,
                cluster_id,
                position: Vec2::new(pos.x, pos.z),
                velocity: Vec2::new(0.0, 0.0),
                guild_id: None,
                party_id: None,
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
            .map(|&(entity_id, cluster_id, pos)| PlayerInfo {
                player_id: entity_id,
                cluster_id,
                position: Vec2::new(pos.x, pos.z),
                velocity: Vec2::new(0.0, 0.0),
                guild_id: None,
                party_id: None,
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

        // Consume assignments from the model and drive migrations.
        let assignments = self.model.compute_entity_assignments(&view);
        self.migration_state.advance_tick();

        // Build a map of current cluster assignment from the view for comparison.
        let mut current_assignments: HashMap<Uuid, Uuid> = HashMap::new();
        for (entity_id, cluster_id, _) in &entity_data {
            current_assignments.insert(*entity_id, *cluster_id);
        }

        for (entity_id, desired_cluster) in assignments {
            if let Some(&current_cluster) = current_assignments.get(&entity_id) {
                if desired_cluster != current_cluster {
                    // Decision is to migrate this entity.
                    if self.migration_state.can_migrate(entity_id) {
                        self.migration_state.record_migration(entity_id);
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

    /// Snapshot of cluster geometry from the spatial index (for visualization / debugging).
    pub fn snapshot_for_view(&self) -> Vec<arcane_core::ClusterGeometry> {
        self.spatial_index.snapshot_for_view()
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

    /// Mark an entity as migrated.
    fn record_migration(&mut self, entity_id: Uuid) {
        self.last_migrated.insert(entity_id, self.current_tick);
        self.in_flight_count += 1;
    }

    /// Decrement in-flight count (call when migration completes or is rejected).
    /// Reserved for future use when the full two-step migration pipeline tracks completion.
    #[allow(dead_code)]
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
        state.record_migration(entity);
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
            state.record_migration(entity);
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

        state.record_migration(entity);
        assert_eq!(state.in_flight_count, 1);

        state.complete_migration();
        assert_eq!(state.in_flight_count, 0);
    }
}
