//! Redis-backed inbound replication subscriber for neighbor cluster deltas.
//!
//! Responsibilities:
//! - subscribe to each neighbor topic (`arcane:replication:<cluster_id>`)
//! - parse incoming JSON payload into `EntityStateDelta`
//! - forward valid deltas to the cluster run loop via `std::sync::mpsc::Sender`
//!
//! This module is intentionally narrow: no topology decisions and no state merging.

use std::sync::mpsc::Sender;
use std::thread;

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
    if neighbor_ids.is_empty() {
        return;
    }
    thread::spawn(move || {
        let client = match redis::Client::open(redis_url.as_str()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("neighbor subscriber: Redis open failed: {}", e);
                return;
            }
        };
        let mut conn = match client.get_connection() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("neighbor subscriber: Redis connection failed: {}", e);
                return;
            }
        };
        let mut pubsub = conn.as_pubsub();
        for nid in &neighbor_ids {
            let topic = format!("arcane:replication:{}", nid);
            if pubsub.subscribe(&topic).is_err() {
                eprintln!("neighbor subscriber: subscribe {} failed", topic);
            }
        }
        eprintln!("subscribed to {} neighbor topic(s)", neighbor_ids.len());
        loop {
            match pubsub.get_message() {
                Ok(msg) => {
                    let payload: String = match msg.get_payload() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if let Some(delta) = parse_delta_payload(&payload) {
                        let _ = neighbor_tx.send(delta);
                    }
                }
                Err(e) => {
                    eprintln!("neighbor subscriber: get_message error: {}", e);
                    break;
                }
            }
        }
    });
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
            updated: vec![EntityStateEntry {
                entity_id: Uuid::from_u128(2),
                cluster_id: Uuid::from_u128(3),
                position: Vec3::new(1.0, 2.0, 3.0),
                velocity: Vec3::new(0.1, 0.2, 0.3),
            }],
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
