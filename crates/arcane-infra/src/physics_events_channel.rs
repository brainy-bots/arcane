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

/// Message queued for the publisher thread.
struct PublishMessage {
    source_cluster_id: Uuid,
    routed_ops: Vec<(Uuid, PhysicsEvent)>,
}

/// Publishes physics event batches to target clusters via Redis (non-blocking).
/// Publishing is non-blocking: batches are enqueued on a producer thread via mpsc,
/// which owns the Redis connection and drains the queue.
pub struct PhysicsEventsPublisher {
    tx: std::sync::mpsc::Sender<PublishMessage>,
}

impl PhysicsEventsPublisher {
    pub fn new(redis_url: &str) -> Result<Self, String> {
        let client =
            redis::Client::open(redis_url).map_err(|e| format!("Redis open failed: {}", e))?;
        let (tx, rx) = std::sync::mpsc::channel::<PublishMessage>();

        std::thread::spawn(move || {
            let mut conn = match client.get_connection() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("physics events publisher: initial connection failed: {}", e);
                    return;
                }
            };

            while let Ok(msg) = rx.recv() {
                if msg.routed_ops.is_empty() {
                    continue;
                }

                let mut by_target: std::collections::HashMap<Uuid, Vec<PhysicsEvent>> =
                    std::collections::HashMap::new();
                for (target, event) in msg.routed_ops {
                    by_target.entry(target).or_default().push(event);
                }

                for (target_cluster_id, ops) in by_target {
                    let batch = PhysicsEventBatch {
                        source_cluster_id: msg.source_cluster_id,
                        ops,
                    };
                    if let Ok(payload) = serde_json::to_string(&batch) {
                        let topic = format!("arcane:physics_events:{}", target_cluster_id);
                        let _: Result<i64, redis::RedisError> = redis::cmd("PUBLISH")
                            .arg(&topic)
                            .arg(&payload)
                            .query(&mut conn);
                    }
                }
            }
        });

        Ok(Self { tx })
    }

    /// Enqueue routed ops for non-blocking publication.
    /// `routed_ops` is `(target_cluster_id, PhysicsEvent)`.
    /// Returns immediately without waiting on Redis.
    pub fn publish(
        &self,
        source_cluster_id: Uuid,
        routed_ops: Vec<(Uuid, PhysicsEvent)>,
    ) -> Result<(), String> {
        if routed_ops.is_empty() {
            return Ok(());
        }
        self.tx
            .send(PublishMessage {
                source_cluster_id,
                routed_ops,
            })
            .map_err(|_| "publisher thread dead".to_string())
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
    use std::time::Instant;

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

    #[test]
    fn publish_returns_without_blocking_even_without_redis() {
        // Create a publisher with an unreachable Redis URL.
        // The publisher thread's connection will fail, but enqueue should still work.
        let publisher = match PhysicsEventsPublisher::new("redis://127.0.0.1:16379") {
            Ok(p) => p,
            Err(_) => return, // Skip if we can't create the publisher.
        };

        let ops = vec![(
            Uuid::from_u128(2),
            PhysicsEvent {
                target_entity_id: Uuid::from_u128(1),
                op: PhysicsOp::Wake,
            },
        )];

        let start = Instant::now();
        let result = publisher.publish(Uuid::from_u128(1), ops);
        let elapsed = start.elapsed();

        // publish() should return promptly (< 10ms).
        assert!(
            elapsed.as_millis() < 10,
            "publish() took too long: {:?}",
            elapsed
        );
        // The enqueue itself should succeed (thread is running).
        assert!(result.is_ok(), "publish should enqueue successfully");
    }

    #[test]
    fn publish_multiple_batches_without_blocking() {
        let publisher = match PhysicsEventsPublisher::new("redis://127.0.0.1:16379") {
            Ok(p) => p,
            Err(_) => return,
        };

        let start = Instant::now();
        for i in 0..50 {
            let ops = vec![(
                Uuid::from_u128(2),
                PhysicsEvent {
                    target_entity_id: Uuid::from_u128(i),
                    op: PhysicsOp::Wake,
                },
            )];
            let _ = publisher.publish(Uuid::from_u128(1), ops);
        }
        let elapsed = start.elapsed();

        // 50 non-blocking enqueues should complete very quickly.
        assert!(
            elapsed.as_millis() < 100,
            "50 publishes took too long: {:?}",
            elapsed
        );
    }
}
