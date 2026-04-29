//! ClusterManager (IN-01) — central coordinator.

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

/// Guardrails for decision execution. All thresholds are per-cycle or per-pair.
#[derive(Clone, Debug)]
pub struct ExecutionConfig {
    /// Minimum model confidence to execute a decision (0.0–1.0).
    pub min_confidence: f32,
    /// Ticks to suppress further merges involving the surviving cluster after a merge.
    pub merge_cooldown_ticks: u32,
    /// Ticks to suppress further splits involving either resulting cluster after a split.
    pub split_cooldown_ticks: u32,
    /// Maximum decisions executed per evaluation cycle (merge + split combined).
    pub max_per_cycle: usize,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.7,
            merge_cooldown_ticks: 20,
            split_cooldown_ticks: 30,
            max_per_cycle: 3,
        }
    }
}

/// Central coordinator: assignments, topology, clustering model.
pub struct ClusterManager {
    model: Arc<dyn IClusteringModel>,
    pool: Arc<dyn IServerPool>,
    spatial_index: SpatialIndex,
    /// cluster_id → ServerHandle. One entry per live cluster server.
    servers: HashMap<Uuid, ServerHandle>,
    exec_config: ExecutionConfig,
    /// cluster_id → remaining cooldown ticks after a merge.
    merge_cooldowns: HashMap<Uuid, u32>,
    /// cluster_id → remaining cooldown ticks after a split.
    split_cooldowns: HashMap<Uuid, u32>,
}

impl ClusterManager {
    pub fn new(
        model: Arc<dyn IClusteringModel>,
        pool: Arc<dyn IServerPool>,
        spatial_index: SpatialIndex,
    ) -> Self {
        Self {
            model,
            pool,
            spatial_index,
            servers: HashMap::new(),
            exec_config: ExecutionConfig::default(),
            merge_cooldowns: HashMap::new(),
            split_cooldowns: HashMap::new(),
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
    pub fn run_evaluation_cycle(&mut self) -> Result<(), String> {
        self.merge_cooldowns
            .values_mut()
            .for_each(|v| *v = v.saturating_sub(1));
        self.merge_cooldowns.retain(|_, v| *v > 0);
        self.split_cooldowns
            .values_mut()
            .for_each(|v| *v = v.saturating_sub(1));
        self.split_cooldowns.retain(|_, v| *v > 0);

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
        if !self.servers.is_empty() {
            return Ok(());
        }
        match self.pool.allocate() {
            Ok(handle) => {
                self.servers.insert(handle.server_id, handle);
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
        self.servers.len() as u32
    }

    /// Snapshot of cluster geometry from the spatial index (for visualization / debugging).
    pub fn snapshot_for_view(&self) -> Vec<arcane_core::ClusterGeometry> {
        self.spatial_index.snapshot_for_view()
    }
}
