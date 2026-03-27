//! IReplicationChannel (IF-03) — cluster-to-cluster state broadcast (pub/sub).
//!
//! Defines the delta payload shape shared between cluster runtime and transport adapters.
//! Infra components (`ReplicationChannelManager`, Redis adapter, neighbor subscribers) exchange
//! `EntityStateDelta` values defined here.

use crate::types::Vec3;
use uuid::Uuid;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_state_delta_serde_roundtrip() {
        let cid = Uuid::nil();
        let eid = Uuid::max();
        let delta = EntityStateDelta {
            source_cluster_id: cid,
            seq: 7,
            tick: 100,
            timestamp: 1.5,
            updated: vec![EntityStateEntry {
                entity_id: eid,
                cluster_id: cid,
                position: Vec3::new(1.0, 2.0, 3.0),
                velocity: Vec3::new(0.1, 0.0, -0.2),
            }],
            removed: vec![Uuid::from_u128(1)],
        };
        let json = serde_json::to_string(&delta).unwrap();
        let back: EntityStateDelta = serde_json::from_str(&json).unwrap();
        assert_eq!(delta.source_cluster_id, back.source_cluster_id);
        assert_eq!(delta.seq, back.seq);
        assert_eq!(delta.updated.len(), back.updated.len());
        assert_eq!(delta.updated[0].entity_id, back.updated[0].entity_id);
        assert_eq!(delta.updated[0].position, back.updated[0].position);
        assert_eq!(delta.removed, back.removed);
    }
}
