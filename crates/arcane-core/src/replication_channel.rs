//! IReplicationChannel (IF-03) — cluster-to-cluster state broadcast (pub/sub).
//!
//! Defines the delta payload shape shared between cluster runtime and transport adapters.
//! Infra components (`ReplicationChannelManager`, Redis adapter, neighbor subscribers) exchange
//! `EntityStateDelta` values defined here.
//!
//! # Four-bucket model
//!
//! [`EntityStateEntry`] maps to the **v1 four-bucket** entity state model (spine, replicated
//! simulation JSON, cluster-local JSON, SpacetimeDB durable). See
//! `docs/architecture/four-bucket-state-model.md` in the repository.

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

/// One entity on the cluster following the **four-bucket** model (see repo doc
/// `docs/architecture/four-bucket-state-model.md`).
///
/// | Bucket | Fields |
/// |--------|--------|
/// | **1 — Spine** (routing + pose) | `entity_id`, `cluster_id`, `position`, `velocity` |
/// | **2 — Replicated simulation** | [`Self::user_data`] (JSON); on Redis / reference WebSocket when not null |
/// | **3 — Cluster-local** | [`Self::local_data`] — **never** serialized into [`EntityStateDelta`]; not trusted from clients |
/// | **4 — Durable** | SpacetimeDB tables / reducers — not stored on this struct |
///
/// **Wire:** `entity_id`, `cluster_id`, `position`, `velocity`, and `user_data` (omitted when null).
/// `local_data` uses [`serde::Serialize`] with skip — it does not cross the replication mesh.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EntityStateEntry {
    pub entity_id: Uuid,
    /// Cluster that owns this entity (for client colorization and ownership display).
    pub cluster_id: Uuid,
    pub position: Vec3,
    pub velocity: Vec3,
    /// **Bucket 2** — replicated game JSON (neighbors, clients). Default `null`; omitted when null.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub user_data: serde_json::Value,
    /// **Bucket 3** — cluster process only; never sent on [`EntityStateDelta`].
    ///
    /// **`skip_deserializing`:** replication JSON must never hydrate this field from the wire (neighbors,
    /// Redis, or crafted payloads). Only this process may set `local_data` in memory after deserialize.
    #[serde(default, skip_serializing, skip_deserializing)]
    pub local_data: serde_json::Value,
}

impl EntityStateEntry {
    pub fn new(entity_id: Uuid, cluster_id: Uuid, position: Vec3, velocity: Vec3) -> Self {
        Self {
            entity_id,
            cluster_id,
            position,
            velocity,
            user_data: serde_json::Value::Null,
            local_data: serde_json::Value::Null,
        }
    }
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
            updated: vec![EntityStateEntry::new(
                eid,
                cid,
                Vec3::new(1.0, 2.0, 3.0),
                Vec3::new(0.1, 0.0, -0.2),
            )],
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

    #[test]
    fn entity_state_entry_user_data_roundtrip() {
        let cid = Uuid::nil();
        let eid = Uuid::from_u128(42);
        let mut e =
            EntityStateEntry::new(eid, cid, Vec3::new(1.0, 0.0, 2.0), Vec3::new(0.0, 0.0, 0.0));
        e.user_data = serde_json::json!({"kind": "projectile", "owner": "a"});
        let json = serde_json::to_string(&e).unwrap();
        let back: EntityStateEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.user_data, e.user_data);
    }

    #[test]
    fn entity_state_entry_local_data_not_on_replication_wire() {
        let cid = Uuid::nil();
        let eid = Uuid::from_u128(7);
        let mut e =
            EntityStateEntry::new(eid, cid, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0));
        e.user_data = serde_json::json!({"visible": true});
        e.local_data = serde_json::json!({"cooldown_s": 2.5});

        let delta = EntityStateDelta {
            source_cluster_id: cid,
            seq: 1,
            tick: 1,
            timestamp: 0.0,
            updated: vec![e],
            removed: vec![],
        };
        let json = serde_json::to_string(&delta).unwrap();
        assert!(
            !json.contains("cooldown"),
            "local_data must not appear in replication JSON: {}",
            json
        );

        let back: EntityStateDelta = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.updated[0].user_data,
            serde_json::json!({"visible": true})
        );
        assert!(
            back.updated[0].local_data.is_null(),
            "after wire roundtrip local_data is absent (default null)"
        );
    }

    #[test]
    fn entity_state_entry_local_data_not_accepted_from_replication_json() {
        // Malicious or buggy neighbor must not inject bucket-3 state via the wire.
        let json = r#"{
            "source_cluster_id":"00000000-0000-0000-0000-000000000001",
            "seq":1,"tick":1,"timestamp":0.0,
            "updated":[{
                "entity_id":"00000000-0000-0000-0000-000000000002",
                "cluster_id":"00000000-0000-0000-0000-000000000001",
                "position":{"x":1.0,"y":0.0,"z":0.0},
                "velocity":{"x":0.0,"y":0.0,"z":0.0},
                "user_data":{"ok":true},
                "local_data":{"injected":true}
            }],
            "removed":[]
        }"#;
        let delta: EntityStateDelta = serde_json::from_str(json).unwrap();
        assert_eq!(delta.updated[0].user_data, serde_json::json!({"ok": true}));
        assert!(
            delta.updated[0].local_data.is_null(),
            "local_data from JSON must be ignored (skip_deserializing)"
        );
    }
}
