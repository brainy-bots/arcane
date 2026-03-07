//! IReplicationChannel (IF-03) — cluster-to-cluster state broadcast (pub/sub).

use uuid::Uuid;
use crate::types::Vec3;

/// Configuration for a replication channel (one neighbor).
#[derive(Clone, Debug)]
pub struct ChannelConfig {
    pub observation_radius: f64,
    pub max_queue_depth: usize,
    pub send_interval_ms: u32,
    pub compression_enabled: bool,
}

/// Entity state delta sent to a neighbor. Fire-and-forget; no ack.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EntityStateDelta {
    pub source_cluster_id: Uuid,
    pub seq: i64,
    pub tick: u64,
    pub timestamp: f64,
    pub updated: Vec<EntityStateEntry>,
    pub removed: Vec<Uuid>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EntityStateEntry {
    pub entity_id: Uuid,
    /// Cluster that owns this entity (for client colorization and ownership display).
    pub cluster_id: Uuid,
    pub position: Vec3,
    pub velocity: Vec3,
}

/// Reason for closing a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    NeighborDepartured,
    ClustersMerged,
    Shutdown,
}

/// Contract for publishing/subscribing to a neighbor's state. One instance per neighbor.
pub trait IReplicationChannel: Send + Sync {
    /// Enqueue a delta for transmission. Non-blocking; may drop if queue full.
    fn send(&self, delta: EntityStateDelta);

    /// Close the channel and flush pending sends.
    fn close(&self, reason: CloseReason);
}
