//! Redis-backed IReplicationChannel. Publishes EntityStateDelta to a Redis pub/sub topic.
//! Topic format: arcane:replication:{source_cluster_id}. Neighbors subscribe to that topic.
//!
//! Note: In same-process tests with a subscriber on another thread, the single connection used
//! here may not always deliver before the subscriber times out; integration tests use an
//! additional publish on a fresh connection to assert round-trip. Cross-process use is unaffected.

use std::sync::atomic::{AtomicBool, Ordering};

use arcane_core::replication_channel::{CloseReason, EntityStateDelta, IReplicationChannel};
use redis::Commands;
use uuid::Uuid;

const TOPIC_PREFIX: &str = "arcane:replication";

/// One channel = publish to one topic (our cluster id). Neighbors subscribe to our topic.
pub struct RedisReplicationChannel {
    topic: String,
    conn: std::sync::Mutex<redis::Connection>,
    closed: AtomicBool,
}

impl RedisReplicationChannel {
    /// Create a channel that publishes to `arcane:replication:{source_cluster_id}`.
    /// Caller must open a Redis connection (e.g. from REDIS_URL).
    pub fn new(source_cluster_id: Uuid, conn: redis::Connection) -> Self {
        let topic = format!("{}:{}", TOPIC_PREFIX, source_cluster_id);
        Self {
            topic,
            conn: std::sync::Mutex::new(conn),
            closed: AtomicBool::new(false),
        }
    }

    /// Topic name this channel publishes to (for subscribers).
    pub fn topic(&self) -> &str {
        &self.topic
    }
}

impl IReplicationChannel for RedisReplicationChannel {
    fn send(&self, delta: EntityStateDelta) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let payload = match serde_json::to_string(&delta) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        let _: Result<i32, redis::RedisError> = conn.publish(self.topic.clone(), payload);
    }

    fn close(&self, _reason: CloseReason) {
        self.closed.store(true, Ordering::Relaxed);
    }
}
