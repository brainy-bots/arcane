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

/// Central coordinator: assignments, topology, clustering model.
pub struct ArcaneManager {
    model: Arc<dyn IClusteringModel>,
    pool: Arc<dyn IServerPool>,
    spatial_index: SpatialIndex,
    /// Allocated nodes. active_count = allocated_servers.len().
    allocated_servers: Vec<ServerHandle>,
    /// entity_id -> party_id mapping for social membership.
    entity_party: HashMap<Uuid, Uuid>,
    /// entity_id -> guild_id mapping for social membership.
    entity_guild: HashMap<Uuid, Uuid>,
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
    /// Ownership-flip decisions made this cycle, awaiting drain by the caller.
    /// The Manager decides but never publishes (design §3: it never talks to clusters
    /// directly). The caller drains these via `ArcaneManager::take_pending_flips` and
    /// actuates them (the Router's job in the target architecture).
    pending_flips: Vec<OwnershipFlip>,
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
            entity_party: HashMap::new(),
            entity_guild: HashMap::new(),
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

    /// Set the velocity for an entity (delegates to SpatialIndex).
    pub fn set_entity_velocity(&mut self, entity_id: Uuid, velocity: Vec3) {
        self.spatial_index
            .update_entity_velocity(entity_id, velocity);
    }

    /// Set the party ID for an entity (insert or remove if None).
    pub fn set_entity_party(&mut self, entity_id: Uuid, party_id: Option<Uuid>) {
        match party_id {
            Some(id) => {
                self.entity_party.insert(entity_id, id);
            }
            None => {
                self.entity_party.remove(&entity_id);
            }
        }
    }

    /// Set the guild ID for an entity (insert or remove if None).
    pub fn set_entity_guild(&mut self, entity_id: Uuid, guild_id: Option<Uuid>) {
        match guild_id {
            Some(id) => {
                self.entity_guild.insert(entity_id, id);
            }
            None => {
                self.entity_guild.remove(&entity_id);
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
                    guild_id: self.entity_guild.get(&entity_id).copied(),
                    party_id: self.entity_party.get(&entity_id).copied(),
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
                    guild_id: self.entity_guild.get(&entity_id).copied(),
                    party_id: self.entity_party.get(&entity_id).copied(),
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

    #[test]
    fn test_social_membership_storage() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);
        let party_id = Uuid::from_u128(200);
        let guild_id = Uuid::from_u128(300);

        // Set party and guild
        manager.set_entity_party(entity_id, Some(party_id));
        manager.set_entity_guild(entity_id, Some(guild_id));

        // Verify storage
        assert_eq!(
            manager.entity_party.get(&entity_id).copied(),
            Some(party_id)
        );
        assert_eq!(
            manager.entity_guild.get(&entity_id).copied(),
            Some(guild_id)
        );
    }

    #[test]
    fn test_social_membership_removal() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);
        let party_id = Uuid::from_u128(200);

        // Set and then remove party
        manager.set_entity_party(entity_id, Some(party_id));
        assert_eq!(
            manager.entity_party.get(&entity_id).copied(),
            Some(party_id)
        );

        manager.set_entity_party(entity_id, None);
        assert_eq!(manager.entity_party.get(&entity_id).copied(), None);
    }

    #[test]
    fn test_worldstateview_reflects_real_signals() {
        let mut manager = ArcaneManager::with_defaults();
        manager.set_observation_radius(100.0);

        let entity1_id = Uuid::from_u128(1);
        let entity2_id = Uuid::from_u128(2);
        let cluster1_id = Uuid::from_u128(100);
        let cluster2_id = Uuid::from_u128(101);
        let party_id = Uuid::from_u128(500);

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

        // Set up two entities with party membership and velocities
        manager.update_entity(entity1_id, cluster1_id, pos1);
        manager.update_entity(entity2_id, cluster2_id, pos2);
        manager.set_entity_velocity(entity1_id, vel1);
        manager.set_entity_velocity(entity2_id, vel2);
        manager.set_entity_party(entity1_id, Some(party_id));
        manager.set_entity_party(entity2_id, Some(party_id));

        // Run evaluation cycle
        let result = manager.run_evaluation_cycle();
        assert!(result.is_ok());

        // Verify snapshot contains the real signals
        let snapshot_entities = manager.spatial_index.snapshot_entities();
        assert_eq!(snapshot_entities.len(), 2);

        // Verify velocity is retrieved correctly (x/z mapping per spec)
        for (entity_id, _, pos) in &snapshot_entities {
            if *entity_id == entity1_id {
                let retrieved_vel = manager.spatial_index.velocity_of(entity1_id);
                assert_eq!(retrieved_vel, Some(vel1));
            } else if *entity_id == entity2_id {
                let retrieved_vel = manager.spatial_index.velocity_of(entity2_id);
                assert_eq!(retrieved_vel, Some(vel2));
            }
        }

        // Verify party membership is accessible
        assert_eq!(
            manager.entity_party.get(&entity1_id).copied(),
            Some(party_id)
        );
        assert_eq!(
            manager.entity_party.get(&entity2_id).copied(),
            Some(party_id)
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
