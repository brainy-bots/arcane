//! World-state view types consumed by the manager decision path.
//!
//! `WorldStateView` (with `ClusterInfo` / `PlayerInfo`) is the read-only
//! snapshot the manager assembles from the spatial index each cycle. The
//! interaction-graph partitioner (`arcane_infra::manager::build_partition_decisions`)
//! reads it.
//!
//! The old merge/split decision interface (`IClusteringModel`,
//! `ClusterDecision`, ...) lived here too. It was the pre-partition design:
//! the manager computed and DISCARDED its output, so it was removed
//! (arcane#291/#292). The swap seam moved to the interaction predictor
//! (`arcane_affinity::predictor::InteractionPredictor`); see #292.

use crate::types::Vec2;
use uuid::Uuid;

/// View of world state passed to the clustering model. ArcaneManager maintains this from SpacetimeDB subscriptions.
#[derive(Clone, Debug)]
pub struct WorldStateView {
    /// Monotonic timestamp (seconds since epoch) of the snapshot.
    pub timestamp: f64,
    /// Maximum wall-clock ms the model may spend on this evaluation cycle.
    pub evaluation_budget_ms: u32,
    /// All clusters visible in the live view at this instant.
    pub clusters: Vec<ClusterInfo>,
    /// All players visible in the live view at this instant.
    pub players: Vec<PlayerInfo>,
}

/// Per-cluster info in the live view.
#[derive(Clone, Debug)]
pub struct ClusterInfo {
    /// Unique identifier for this cluster.
    pub cluster_id: Uuid,
    /// Hostname of the server serving this cluster.
    pub server_host: String,
    /// Player UUIDs assigned to this cluster.
    pub player_ids: Vec<Uuid>,
    /// Cached player count (derived from `player_ids.len()`).
    pub player_count: u32,
    /// CPU utilisation as a percentage (0.0–100.0).
    pub cpu_pct: f32,
    /// Spatial centroid of the cluster's players in world coordinates.
    pub centroid: Vec2,
    /// Maximum distance of any player from the centroid.
    pub spread_radius: f32,
    /// Outbound cross-cluster RPCs per second from this cluster.
    pub rpc_rate_out: f32,
}

/// Per-player info in the live view.
#[derive(Clone, Debug)]
pub struct PlayerInfo {
    /// Unique identifier for this player.
    pub player_id: Uuid,
    /// Cluster the player is currently assigned to.
    pub cluster_id: Uuid,
    /// Current world-space position.
    pub position: Vec2,
    /// Current velocity vector (units/second).
    pub velocity: Vec2,
}
