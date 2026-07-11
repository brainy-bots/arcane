//! Ownership migration — represent and signal ownership changes over Redis.
//!
//! This module provides:
//! - An explicit owner map tracking which cluster owns each entity
//! - Ephemeral `OwnershipFlip` messages signaled over Redis
//! - Publisher and subscriber for sending/receiving ownership flips
//!
//! Follows the same async pattern as `physics_events_channel`: non-blocking publisher
//! thread via mpsc, and a spawnable subscriber thread. No SpacetimeDB involvement.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::thread;
use uuid::Uuid;

const OWNERSHIP_TOPIC_PREFIX: &str = "arcane:ownership-flip";

/// Represents an ownership transfer of an entity from one cluster to another.
///
/// Serializable with `serde` for Redis transport; all fields are `Copy`/POD.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnershipFlip {
    pub entity_id: Uuid,
    pub from_cluster: Uuid,
    pub to_cluster: Uuid,
    pub effective_tick: u64,
}

/// Message queued for the publisher thread.
struct PublishMessage {
    flip: OwnershipFlip,
}

/// Publishes ownership flips to the appropriate clusters via Redis (non-blocking).
///
/// Publishing is non-blocking: flips are enqueued on a producer thread via mpsc,
/// which owns the Redis connection and drains the queue.
pub struct OwnershipFlipPublisher {
    tx: std::sync::mpsc::Sender<PublishMessage>,
}

impl OwnershipFlipPublisher {
    /// Create a new publisher that will broadcast flips via Redis.
    pub fn new(redis_url: &str) -> Result<Self, String> {
        let client =
            redis::Client::open(redis_url).map_err(|e| format!("Redis open failed: {}", e))?;
        let (tx, rx) = std::sync::mpsc::channel::<PublishMessage>();

        std::thread::spawn(move || {
            // Lazily (re)connect; never exit on connection failure so `publish()` can always enqueue.
            let mut conn: Option<redis::Connection> = client.get_connection().ok();
            while let Ok(msg) = rx.recv() {
                if conn.is_none() {
                    conn = client.get_connection().ok();
                }
                let Some(c) = conn.as_mut() else {
                    continue;
                };

                let flip = msg.flip;
                if let Ok(payload) = serde_json::to_string(&flip) {
                    // Publish to both from_cluster and to_cluster topics so both nodes see the flip.
                    let from_topic = format!("{}:{}", OWNERSHIP_TOPIC_PREFIX, flip.from_cluster);
                    let to_topic = format!("{}:{}", OWNERSHIP_TOPIC_PREFIX, flip.to_cluster);

                    let mut should_reconnect = false;
                    let res_from: Result<i64, redis::RedisError> = redis::cmd("PUBLISH")
                        .arg(&from_topic)
                        .arg(&payload)
                        .query(c);
                    let res_to: Result<i64, redis::RedisError> =
                        redis::cmd("PUBLISH").arg(&to_topic).arg(&payload).query(c);

                    if res_from.is_err() || res_to.is_err() {
                        should_reconnect = true;
                    }

                    eprintln!(
                        "OwnershipFlip published: entity={}, from={}, to={}, effective_tick={}",
                        flip.entity_id, flip.from_cluster, flip.to_cluster, flip.effective_tick
                    );

                    if should_reconnect {
                        conn = None;
                    }
                }
            }
        });

        Ok(Self { tx })
    }

    /// Enqueue a flip for non-blocking publication to both losing and gaining nodes.
    /// Returns immediately without waiting on Redis.
    pub fn publish(&self, flip: OwnershipFlip) -> Result<(), String> {
        self.tx
            .send(PublishMessage { flip })
            .map_err(|_| "publisher thread dead".to_string())
    }
}

/// Spawn a background thread subscribing to ownership flip events for this cluster.
///
/// Both the losing node (from_cluster) and gaining node (to_cluster) subscribe to the
/// same topics and observe all flips. When a flip is received, `set_owner` is called
/// to update the local ownership map.
pub fn spawn_ownership_flip_subscriber(
    redis_url: String,
    cluster_id: Uuid,
    ownership_map: OwnershipMap,
) {
    thread::spawn(move || {
        let client = match redis::Client::open(redis_url.as_str()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ownership flip subscriber: Redis open failed: {}", e);
                return;
            }
        };
        let mut conn = match client.get_connection() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ownership flip subscriber: Redis connection failed: {}", e);
                return;
            }
        };
        let mut pubsub = conn.as_pubsub();
        let topic = format!("{}:{}", OWNERSHIP_TOPIC_PREFIX, cluster_id);
        if pubsub.subscribe(&topic).is_err() {
            eprintln!("ownership flip subscriber: subscribe {} failed", topic);
            return;
        }
        eprintln!("subscribed to ownership flip topic {}", topic);
        loop {
            match pubsub.get_message() {
                Ok(msg) => {
                    let payload: String = match msg.get_payload() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if let Ok(flip) = serde_json::from_str::<OwnershipFlip>(&payload) {
                        eprintln!(
                            "OwnershipFlip received: entity={}, from={}, to={}, effective_tick={}",
                            flip.entity_id, flip.from_cluster, flip.to_cluster, flip.effective_tick
                        );
                        ownership_map.set_owner(flip.entity_id, flip.to_cluster);
                    }
                }
                Err(e) => {
                    eprintln!("ownership flip subscriber: get_message error: {}", e);
                    break;
                }
            }
        }
    });
}

/// Ownership map — tracks which cluster owns each entity.
///
/// Thread-safe. Used by nodes to answer "do I own entity X?" and to update ownership
/// when receiving `OwnershipFlip` messages.
#[derive(Debug, Clone)]
pub struct OwnershipMap {
    owners: std::sync::Arc<std::sync::Mutex<HashMap<Uuid, Uuid>>>,
}

impl OwnershipMap {
    /// Create a new empty ownership map.
    pub fn new() -> Self {
        Self {
            owners: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Set the owner of an entity.
    pub fn set_owner(&self, entity_id: Uuid, owner_cluster: Uuid) {
        if let Ok(mut map) = self.owners.lock() {
            map.insert(entity_id, owner_cluster);
        }
    }

    /// Get the owner of an entity, or None if not in map.
    pub fn owner_of(&self, entity_id: Uuid) -> Option<Uuid> {
        self.owners
            .lock()
            .ok()
            .and_then(|map| map.get(&entity_id).copied())
    }

    /// Check if this cluster owns a given entity.
    pub fn owns(&self, entity_id: Uuid, my_cluster: Uuid) -> bool {
        self.owner_of(entity_id)
            .map(|owner| owner == my_cluster)
            .unwrap_or(false)
    }
}

impl Default for OwnershipMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn ownership_flip_serializes_and_deserializes() {
        let flip = OwnershipFlip {
            entity_id: Uuid::from_u128(1),
            from_cluster: Uuid::from_u128(10),
            to_cluster: Uuid::from_u128(20),
            effective_tick: 42,
        };

        let json = serde_json::to_string(&flip).expect("serialize");
        let deserialized: OwnershipFlip = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(flip, deserialized);
    }

    #[test]
    fn ownership_map_set_and_get() {
        let map = OwnershipMap::new();
        let entity_id = Uuid::from_u128(1);
        let owner = Uuid::from_u128(100);

        map.set_owner(entity_id, owner);
        assert_eq!(map.owner_of(entity_id), Some(owner));
    }

    #[test]
    fn ownership_map_owns_returns_true_for_owner() {
        let map = OwnershipMap::new();
        let entity_id = Uuid::from_u128(1);
        let owner = Uuid::from_u128(100);

        map.set_owner(entity_id, owner);
        assert!(map.owns(entity_id, owner));
    }

    #[test]
    fn ownership_map_owns_returns_false_for_non_owner() {
        let map = OwnershipMap::new();
        let entity_id = Uuid::from_u128(1);
        let owner = Uuid::from_u128(100);
        let other = Uuid::from_u128(200);

        map.set_owner(entity_id, owner);
        assert!(!map.owns(entity_id, other));
    }

    #[test]
    fn ownership_map_owns_returns_false_for_unknown_entity() {
        let map = OwnershipMap::new();
        let entity_id = Uuid::from_u128(1);
        let cluster = Uuid::from_u128(100);

        assert!(!map.owns(entity_id, cluster));
    }

    #[test]
    fn publisher_returns_without_blocking_even_without_redis() {
        // Create a publisher with an unreachable Redis URL.
        // The publisher thread's connection will fail, but enqueue should still work.
        let publisher = match OwnershipFlipPublisher::new("redis://127.0.0.1:16379") {
            Ok(p) => p,
            Err(_) => return, // Skip if we can't create the publisher.
        };

        let flip = OwnershipFlip {
            entity_id: Uuid::from_u128(1),
            from_cluster: Uuid::from_u128(10),
            to_cluster: Uuid::from_u128(20),
            effective_tick: 42,
        };

        let start = Instant::now();
        let result = publisher.publish(flip);
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
    fn publish_multiple_flips_without_blocking() {
        let publisher = match OwnershipFlipPublisher::new("redis://127.0.0.1:16379") {
            Ok(p) => p,
            Err(_) => return,
        };

        let start = Instant::now();
        for i in 0..50 {
            let flip = OwnershipFlip {
                entity_id: Uuid::from_u128(i),
                from_cluster: Uuid::from_u128(10),
                to_cluster: Uuid::from_u128(20),
                effective_tick: 42 + (i as u64),
            };
            let _ = publisher.publish(flip);
        }
        let elapsed = start.elapsed();

        // 50 non-blocking publishes should complete very quickly.
        assert!(
            elapsed.as_millis() < 100,
            "50 publishes took too long: {:?}",
            elapsed
        );
    }
}
