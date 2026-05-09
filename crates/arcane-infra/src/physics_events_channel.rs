//! Redis pub/sub transport for cross-cluster physics events.
//!
//! Follows the same pattern as [`crate::neighbor_subscriber`]: thread-spawned
//! subscriber, JSON over Redis pub/sub, mpsc forwarding.
//!
//! Topic: `arcane:physics_events:<cluster_uuid>` (point-to-point — each cluster
//! subscribes to its own topic).

use std::sync::mpsc::Sender;
use std::thread;

use arcane_core::physics_events::{PhysicsEvent, PhysicsEventBatch};
use uuid::Uuid;

/// Publishes physics event batches to target clusters via Redis.
pub struct PhysicsEventsPublisher {
    client: redis::Client,
}

impl PhysicsEventsPublisher {
    pub fn new(redis_url: &str) -> Result<Self, String> {
        let client =
            redis::Client::open(redis_url).map_err(|e| format!("Redis open failed: {}", e))?;
        Ok(Self { client })
    }

    /// Group routed ops by target cluster and publish each batch.
    /// `routed_ops` is `(target_cluster_id, PhysicsEvent)`.
    pub fn publish(
        &self,
        source_cluster_id: Uuid,
        routed_ops: Vec<(Uuid, PhysicsEvent)>,
    ) -> Result<(), String> {
        if routed_ops.is_empty() {
            return Ok(());
        }
        let mut conn = self
            .client
            .get_connection()
            .map_err(|e| format!("Redis connection failed: {}", e))?;

        let mut by_target: std::collections::HashMap<Uuid, Vec<PhysicsEvent>> =
            std::collections::HashMap::new();
        for (target, event) in routed_ops {
            by_target.entry(target).or_default().push(event);
        }

        for (target_cluster_id, ops) in by_target {
            let batch = PhysicsEventBatch {
                source_cluster_id,
                ops,
            };
            let payload = serde_json::to_string(&batch)
                .map_err(|e| format!("serialize physics batch: {}", e))?;
            let topic = format!("arcane:physics_events:{}", target_cluster_id);
            redis::cmd("PUBLISH")
                .arg(&topic)
                .arg(&payload)
                .query::<i64>(&mut conn)
                .map_err(|e| format!("Redis PUBLISH failed: {}", e))?;
        }
        Ok(())
    }
}

/// Spawn a background thread subscribing to this cluster's physics events topic.
pub fn spawn_physics_events_subscriber(
    redis_url: String,
    self_cluster_id: Uuid,
    tx: Sender<PhysicsEventBatch>,
) {
    thread::spawn(move || {
        let client = match redis::Client::open(redis_url.as_str()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("physics events subscriber: Redis open failed: {}", e);
                return;
            }
        };
        let mut conn = match client.get_connection() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("physics events subscriber: Redis connection failed: {}", e);
                return;
            }
        };
        let mut pubsub = conn.as_pubsub();
        let topic = format!("arcane:physics_events:{}", self_cluster_id);
        if pubsub.subscribe(&topic).is_err() {
            eprintln!("physics events subscriber: subscribe {} failed", topic);
            return;
        }
        eprintln!("subscribed to physics events topic {}", topic);
        loop {
            match pubsub.get_message() {
                Ok(msg) => {
                    let payload: String = match msg.get_payload() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if let Ok(batch) = serde_json::from_str::<PhysicsEventBatch>(&payload) {
                        let _ = tx.send(batch);
                    }
                }
                Err(e) => {
                    eprintln!("physics events subscriber: get_message error: {}", e);
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_core::physics_events::PhysicsOp;

    #[test]
    fn physics_event_batch_json_roundtrip() {
        let batch = PhysicsEventBatch {
            source_cluster_id: Uuid::from_u128(1),
            ops: vec![
                PhysicsEvent {
                    target_entity_id: Uuid::from_u128(10),
                    op: PhysicsOp::ApplyImpulse {
                        impulse: [1.0, 2.0, 3.0],
                    },
                },
                PhysicsEvent {
                    target_entity_id: Uuid::from_u128(11),
                    op: PhysicsOp::ContactEvent {
                        other_entity_id: Uuid::from_u128(12),
                        started: true,
                    },
                },
                PhysicsEvent {
                    target_entity_id: Uuid::from_u128(13),
                    op: PhysicsOp::Wake,
                },
            ],
        };
        let json = serde_json::to_string(&batch).unwrap();
        let parsed: PhysicsEventBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.source_cluster_id, batch.source_cluster_id);
        assert_eq!(parsed.ops.len(), 3);
    }
}
