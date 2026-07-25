//! Redis-backed inbound replication subscriber for neighbor cluster deltas.
//!
//! Responsibilities:
//! - subscribe to each neighbor topic (`arcane:replication:<cluster_id>`)
//! - parse incoming JSON payload into `EntityStateDelta`
//! - forward valid deltas to the cluster run loop via `std::sync::mpsc::Sender`
//!
//! This module is intentionally narrow: no topology decisions and no state merging.

use std::ops::ControlFlow;
use std::sync::mpsc::Sender;

use arcane_core::replication_channel::EntityStateDelta;
use uuid::Uuid;

fn parse_delta_payload(payload: &str) -> Option<EntityStateDelta> {
    serde_json::from_str::<EntityStateDelta>(payload).ok()
}

pub fn spawn_neighbor_subscriber(
    redis_url: String,
    neighbor_ids: Vec<Uuid>,
    neighbor_tx: Sender<EntityStateDelta>,
) {
    let topics: Vec<String> = neighbor_ids
        .iter()
        .map(|nid| format!("arcane:replication:{}", nid))
        .collect();
    // Resilient loop (arcane#204): reconnect with backoff instead of dying on
    // the first dropped connection. Missed deltas are healed by the
    // publisher's continuous resync cadence.
    crate::pubsub_util::spawn_resilient_subscriber(
        "neighbor subscriber",
        redis_url,
        topics,
        move |payload| {
            if let Some(delta) = parse_delta_payload(&payload) {
                if neighbor_tx.send(delta).is_err() {
                    return ControlFlow::Break(()); // node dropped its receiver
                }
            }
            ControlFlow::Continue(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_core::replication_channel::EntityStateEntry;
    use arcane_core::Vec3;

    #[test]
    fn parse_delta_payload_accepts_valid_json() {
        let delta = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(1),
            seq: 5,
            tick: 42,
            timestamp: 1.23,
            updated: vec![EntityStateEntry::new(
                Uuid::from_u128(2),
                Uuid::from_u128(3),
                Vec3::new(1.0, 2.0, 3.0),
                Vec3::new(0.1, 0.2, 0.3),
            )],
            removed: vec![Uuid::from_u128(4)],
        };
        let payload = serde_json::to_string(&delta).unwrap();
        let parsed = parse_delta_payload(&payload).unwrap();
        assert_eq!(parsed.source_cluster_id, delta.source_cluster_id);
        assert_eq!(parsed.seq, delta.seq);
        assert_eq!(parsed.updated.len(), 1);
        assert_eq!(parsed.removed, delta.removed);
    }

    #[test]
    fn parse_delta_payload_rejects_invalid_json() {
        let parsed = parse_delta_payload("{not-json");
        assert!(parsed.is_none());
    }
}
