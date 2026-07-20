//! Node inbox — the single per-node channel for the Router's state/ownership feed.
//!
//! The Router publishes one frame per tick to each cluster's inbox topic: entity state
//! (with attention tier) and ownership changes. Nodes subscribe to their own inbox.
//!
//! Follows the same pattern as [`crate::physics_events_channel`]: thread-spawned subscriber,
//! JSON over Redis pub/sub, mpsc forwarding. Two implementations: in-memory for deterministic
//! tests and Redis for production.
//!
//! Topic: `arcane:inbox:<cluster_uuid>` (point-to-point — each cluster subscribes to its own topic).

use crate::ownership_migration::OwnershipFlip;
use arcane_affinity::rate_field::RateTier;
use arcane_core::replication_channel::EntityStateEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{mpsc, Mutex};
use std::thread;
use uuid::Uuid;

/// One replicated entity in a node's inbox frame: the state plus the attention tier the Router
/// assigned it. Binary attention v1: `Zero`-tier entities are simply NOT included in a frame;
/// the tier travels so nodes can prepare for the continuous-rate upgrade without a schema change.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplicatedEntity {
    pub entry: EntityStateEntry,
    pub tier: RateTier,
}

/// One frame written by the Router to a node's inbox (design §2.3/§2.4: the node's ONE input).
/// Carries everything the node needs this tick: ownership changes affecting it, and the foreign
/// entities it should represent (with attention tier). Ownership is folded into the same channel
/// as state (design §3) — no separate control channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeInboxFrame {
    /// Router tick this frame was produced at (monotonic per router).
    pub tick: u64,
    /// Ownership change EVENTS relevant to this node (it is from_cluster or
    /// to_cluster). Informational since #289: nodes derive ownership from
    /// `owned` (the record), not from these deltas. Kept for observability
    /// (the multiprocess E2E asserts gate ordering on them).
    pub ownership: Vec<OwnershipFlip>,
    /// Foreign entities this node should replicate/represent, with attention tier.
    pub entities: Vec<ReplicatedEntity>,
    /// #289: the COMPLETE owned set for this cluster — every frame is an
    /// idempotent statement, not a delta. "You own exactly these entities."
    /// A node reconciles its world against it: adopt what appeared, release
    /// what disappeared. Missing a frame is harmless (the next corrects);
    /// restart is harmless (the first frame tells everything).
    ///
    /// `Option` distinguishes "no statement" (None — pre-#289 frame, node
    /// skips reconciliation) from "you own NOTHING" (Some(empty) — a real
    /// statement that must release everything outside spawn grace). A plain
    /// Vec with serde(default) would conflate the two and an old frame
    /// would wrongly drain the node.
    #[serde(default)]
    pub owned: Option<Vec<Uuid>>,
}

const NODE_INBOX_TOPIC_PREFIX: &str = "arcane:inbox";

/// Transport for node inbox frames: the Router publishes to a cluster's inbox; the node
/// subscribes to its own. Implementations: in-memory (deterministic tests) and Redis
/// (topic `arcane:inbox:<cluster_uuid>`). Sans-IO seam — RouterCore and NodeCore logic stay
/// transport-agnostic.
pub trait InboxBus: Send + Sync {
    /// Publish a frame to a cluster's inbox. Non-blocking (enqueue) for the Redis impl.
    fn publish(&self, cluster_id: Uuid, frame: NodeInboxFrame) -> Result<(), String>;
    /// Subscribe to a cluster's inbox. Frames arrive on the returned receiver.
    fn subscribe(&self, cluster_id: Uuid) -> mpsc::Receiver<NodeInboxFrame>;
}

/// In-memory implementation of InboxBus for deterministic tests.
/// Internally tracks subscribers per cluster and broadcasts frames to all.
#[derive(Default)]
pub struct InMemoryInboxBus {
    // Map from cluster_id to list of subscribers (channels).
    subscribers: Mutex<HashMap<Uuid, Vec<mpsc::Sender<NodeInboxFrame>>>>,
}

impl InMemoryInboxBus {
    pub fn new() -> Self {
        Self::default()
    }
}

impl InboxBus for InMemoryInboxBus {
    fn publish(&self, cluster_id: Uuid, frame: NodeInboxFrame) -> Result<(), String> {
        let mut subscribers = self.subscribers.lock().unwrap();
        let subs = subscribers.entry(cluster_id).or_default();

        // Clone frame to each subscriber that still has a live receiver.
        // Drop senders whose receiver hung up (receiver dropped).
        subs.retain(|tx| tx.send(frame.clone()).is_ok());

        Ok(())
    }

    fn subscribe(&self, cluster_id: Uuid) -> mpsc::Receiver<NodeInboxFrame> {
        let (tx, rx) = mpsc::channel();
        let mut subscribers = self.subscribers.lock().unwrap();
        subscribers.entry(cluster_id).or_default().push(tx);
        rx
    }
}

/// Message queued for the Redis publisher thread.
struct PublishMessage {
    cluster_id: Uuid,
    frame: NodeInboxFrame,
}

/// Redis implementation of InboxBus for production use.
/// Publishing is non-blocking: frames are enqueued on a producer thread via mpsc,
/// which owns the Redis connection and drains the queue.
pub struct RedisInboxBus {
    tx: mpsc::Sender<PublishMessage>,
    /// Redis URL, kept so `subscribe` connects to the SAME server the publisher uses.
    redis_url: String,
}

impl RedisInboxBus {
    /// Create a new Redis-backed inbox bus.
    pub fn new(redis_url: &str) -> Result<Self, String> {
        let client =
            redis::Client::open(redis_url).map_err(|e| format!("Redis open failed: {}", e))?;
        let (tx, rx) = mpsc::channel::<PublishMessage>();

        thread::spawn(move || {
            // Lazily (re)connect; never exit on connection failure so `publish()` can always enqueue.
            let mut conn: Option<redis::Connection> = client.get_connection().ok();
            while let Ok(msg) = rx.recv() {
                if conn.is_none() {
                    conn = client.get_connection().ok();
                }
                let Some(c) = conn.as_mut() else {
                    continue;
                };

                if let Ok(payload) = serde_json::to_string(&msg.frame) {
                    let topic = format!("{}:{}", NODE_INBOX_TOPIC_PREFIX, msg.cluster_id);
                    let res: Result<i64, redis::RedisError> =
                        redis::cmd("PUBLISH").arg(&topic).arg(&payload).query(c);
                    if res.is_err() {
                        conn = None;
                    }
                }
            }
        });

        Ok(Self {
            tx,
            redis_url: redis_url.to_string(),
        })
    }
}

impl InboxBus for RedisInboxBus {
    fn publish(&self, cluster_id: Uuid, frame: NodeInboxFrame) -> Result<(), String> {
        self.tx
            .send(PublishMessage { cluster_id, frame })
            .map_err(|_| "publisher thread dead".to_string())
    }

    fn subscribe(&self, cluster_id: Uuid) -> mpsc::Receiver<NodeInboxFrame> {
        let (tx, rx) = mpsc::channel();
        let redis_url = self.redis_url.clone();

        thread::spawn(move || {
            let client = match redis::Client::open(redis_url.as_str()) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("node inbox subscriber: Redis open failed: {}", e);
                    return;
                }
            };
            let mut conn = match client.get_connection() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("node inbox subscriber: Redis connection failed: {}", e);
                    return;
                }
            };
            let mut pubsub = conn.as_pubsub();
            let topic = format!("{}:{}", NODE_INBOX_TOPIC_PREFIX, cluster_id);
            if pubsub.subscribe(&topic).is_err() {
                eprintln!("node inbox subscriber: subscribe {} failed", topic);
                return;
            }
            eprintln!("subscribed to node inbox topic {}", topic);
            loop {
                match pubsub.get_message() {
                    Ok(msg) => {
                        let payload: String = match msg.get_payload() {
                            Ok(p) => p,
                            Err(_) => continue,
                        };
                        if let Ok(frame) = serde_json::from_str::<NodeInboxFrame>(&payload) {
                            let _ = tx.send(frame);
                        }
                    }
                    Err(e) => {
                        eprintln!("node inbox subscriber: get_message error: {}", e);
                        break;
                    }
                }
            }
        });

        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_core::types::Vec3;

    #[test]
    fn roundtrip_single_cluster() {
        // Publish a frame with 1 flip + 2 entities (one Full, one Low tier) to cluster A.
        // A subscriber of A receives it intact; a subscriber of cluster B receives nothing.
        let bus = InMemoryInboxBus::new();

        let cluster_a = Uuid::from_u128(1);
        let cluster_b = Uuid::from_u128(2);

        let rx_a = bus.subscribe(cluster_a);
        let rx_b = bus.subscribe(cluster_b);

        let entity_1 = ReplicatedEntity {
            entry: EntityStateEntry::new(
                Uuid::from_u128(100),
                cluster_a,
                Vec3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                Vec3 {
                    x: 0.1,
                    y: 0.2,
                    z: 0.3,
                },
            ),
            tier: RateTier::Full,
        };
        let entity_2 = ReplicatedEntity {
            entry: EntityStateEntry::new(
                Uuid::from_u128(101),
                cluster_a,
                Vec3 {
                    x: 4.0,
                    y: 5.0,
                    z: 6.0,
                },
                Vec3 {
                    x: 0.4,
                    y: 0.5,
                    z: 0.6,
                },
            ),
            tier: RateTier::Low,
        };

        let flip = OwnershipFlip {
            entity_id: Uuid::from_u128(200),
            from_cluster: cluster_a,
            to_cluster: cluster_b,
            effective_tick: 42,
        };

        let frame = NodeInboxFrame {
            tick: 100,
            ownership: vec![flip],
            entities: vec![entity_1.clone(), entity_2.clone()],
            owned: None,
        };

        bus.publish(cluster_a, frame.clone()).unwrap();

        // Cluster A subscriber receives the frame.
        let received_a = rx_a
            .recv_timeout(std::time::Duration::from_millis(100))
            .unwrap();
        assert_eq!(received_a.tick, 100);
        assert_eq!(received_a.ownership.len(), 1);
        assert_eq!(received_a.ownership[0].entity_id, Uuid::from_u128(200));
        assert_eq!(received_a.entities.len(), 2);
        assert_eq!(received_a.entities[0].entry.entity_id, Uuid::from_u128(100));
        assert_eq!(received_a.entities[1].entry.entity_id, Uuid::from_u128(101));

        // Cluster B subscriber receives nothing.
        let result_b = rx_b.try_recv();
        assert!(
            result_b.is_err(),
            "Cluster B should not receive cluster A's frames"
        );
    }

    #[test]
    fn multi_subscriber_same_cluster() {
        // Two subscribers of the same cluster both receive the frame.
        let bus = InMemoryInboxBus::new();
        let cluster_id = Uuid::from_u128(5);

        let rx1 = bus.subscribe(cluster_id);
        let rx2 = bus.subscribe(cluster_id);

        let frame = NodeInboxFrame {
            tick: 50,
            ownership: vec![],
            entities: vec![],
            owned: None,
        };

        bus.publish(cluster_id, frame.clone()).unwrap();

        let received1 = rx1
            .recv_timeout(std::time::Duration::from_millis(100))
            .unwrap();
        let received2 = rx2
            .recv_timeout(std::time::Duration::from_millis(100))
            .unwrap();

        assert_eq!(received1.tick, 50);
        assert_eq!(received2.tick, 50);
    }

    #[test]
    fn serde_json_roundtrip_and_bucket_safety() {
        // NodeInboxFrame JSON-roundtrips; local_data and client_seq of the inner
        // EntityStateEntry are NOT in the JSON (bucket-3 safety holds through the inbox).
        let entry = EntityStateEntry {
            entity_id: Uuid::from_u128(10),
            cluster_id: Uuid::from_u128(1),
            position: Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            velocity: Vec3 {
                x: 0.1,
                y: 0.2,
                z: 0.3,
            },
            user_data: serde_json::json!({"test": "data"}),
            local_data: serde_json::json!({"secret": "local"}),
            client_seq: 99,
        };

        let entity = ReplicatedEntity {
            entry,
            tier: RateTier::Full,
        };

        let frame = NodeInboxFrame {
            tick: 123,
            ownership: vec![],
            entities: vec![entity],
            owned: None,
        };

        let json = serde_json::to_string(&frame).unwrap();

        // Verify local_data and client_seq are NOT in the serialized JSON.
        assert!(
            !json.contains("local_data"),
            "Serialized JSON should not contain 'local_data'"
        );
        assert!(
            !json.contains("secret"),
            "Serialized JSON should not contain local data content"
        );
        assert!(
            !json.contains("client_seq"),
            "Serialized JSON should not contain 'client_seq'"
        );
        assert!(
            !json.contains("99"),
            "Serialized JSON should not contain client_seq value"
        );

        // Verify the frame round-trips correctly.
        let parsed: NodeInboxFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tick, 123);
        assert_eq!(parsed.entities.len(), 1);
        assert_eq!(parsed.entities[0].entry.entity_id, Uuid::from_u128(10));
        assert_eq!(
            parsed.entities[0].entry.user_data,
            serde_json::json!({"test": "data"})
        );
        // local_data and client_seq should be default-initialized after deserialization.
        assert_eq!(parsed.entities[0].entry.local_data, serde_json::Value::Null);
        assert_eq!(parsed.entities[0].entry.client_seq, 0);
    }

    #[test]
    fn hung_up_subscriber_dropped() {
        // Publish after a receiver is dropped does not error and later subscribers still work.
        let bus = InMemoryInboxBus::new();
        let cluster_id = Uuid::from_u128(7);

        let rx1 = bus.subscribe(cluster_id);
        let rx2 = bus.subscribe(cluster_id);

        // Drop the first receiver.
        drop(rx1);

        let frame = NodeInboxFrame {
            tick: 77,
            ownership: vec![],
            entities: vec![],
            owned: None,
        };

        // Publishing should not error even though one receiver is gone.
        let result = bus.publish(cluster_id, frame.clone());
        assert!(
            result.is_ok(),
            "Publishing should succeed even with dropped receiver"
        );

        // The second subscriber should still receive the frame.
        let received2 = rx2
            .recv_timeout(std::time::Duration::from_millis(100))
            .unwrap();
        assert_eq!(received2.tick, 77);
    }
}
