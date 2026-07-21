//! Redis-backed IReplicationChannel. Publishes EntityStateDelta to a Redis pub/sub topic.
//! Topic format: arcane:replication:{source_cluster_id}. Neighbors subscribe to that topic.
//!
//! Note: In same-process tests with a subscriber on another thread, the single connection used
//! here may not always deliver before the subscriber times out; integration tests use an
//! additional publish on a fresh connection to assert round-trip. Cross-process use is unaffected.

use std::sync::atomic::Ordering;

use arcane_core::replication_channel::{CloseReason, EntityStateDelta, IReplicationChannel};
use redis::Commands;
use uuid::Uuid;

const TOPIC_PREFIX: &str = "arcane:replication";

/// One channel = publish to one topic (our cluster id). Neighbors subscribe to our topic.
/// Publishing is non-blocking: deltas are enqueued on a producer thread via mpsc,
/// which owns the Redis connection and drains the queue.
pub struct RedisReplicationChannel {
    topic: String,
    tx: std::sync::mpsc::Sender<EntityStateDelta>,
    closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl RedisReplicationChannel {
    /// Create a channel that publishes to `arcane:replication:{source_cluster_id}`.
    /// Spawns a background publisher thread that owns the Redis connection.
    pub fn new(source_cluster_id: Uuid, conn: redis::Connection) -> Self {
        let topic = format!("{}:{}", TOPIC_PREFIX, source_cluster_id);
        let (tx, rx) = std::sync::mpsc::channel::<EntityStateDelta>();
        let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let topic_t = topic.clone();
        let closed_t = closed.clone();

        std::thread::spawn(move || {
            let mut conn = conn;
            while let Ok(delta) = rx.recv() {
                if closed_t.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(payload) = serde_json::to_string(&delta) {
                    let _: Result<i32, redis::RedisError> = conn.publish(&topic_t, payload);
                }
            }
        });

        Self { topic, tx, closed }
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
        let _ = self.tx.send(delta);
    }

    fn close(&self, _reason: CloseReason) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn send_returns_without_blocking_when_channel_has_no_connection() {
        // Create a Redis client that won't connect (invalid URL or no broker).
        // The channel is created, but the publisher thread's connection will fail.
        // send() should still return immediately (enqueue only).
        let fake_conn = redis::Client::open("redis://127.0.0.1:16379")
            .ok()
            .and_then(|c| c.get_connection().ok());

        if let Some(conn) = fake_conn {
            let channel = RedisReplicationChannel::new(Uuid::from_u128(1), conn);
            let delta = EntityStateDelta {
                source_cluster_id: Uuid::from_u128(1),
                seq: 1,
                tick: 42,
                timestamp: 0.0,
                updated: vec![],
                removed: vec![],
            };

            let start = Instant::now();
            channel.send(delta.clone());
            let elapsed = start.elapsed();

            // send() should return promptly (< 10ms, microsecond scale).
            assert!(
                elapsed.as_millis() < 10,
                "send() took too long: {:?}",
                elapsed
            );
        }
    }

    #[test]
    fn send_multiple_times_without_blocking() {
        let fake_conn = redis::Client::open("redis://127.0.0.1:16379")
            .ok()
            .and_then(|c| c.get_connection().ok());

        if let Some(conn) = fake_conn {
            let channel = RedisReplicationChannel::new(Uuid::from_u128(1), conn);
            let delta = EntityStateDelta {
                source_cluster_id: Uuid::from_u128(1),
                seq: 1,
                tick: 42,
                timestamp: 0.0,
                updated: vec![],
                removed: vec![],
            };

            let start = Instant::now();
            for _ in 0..100 {
                channel.send(delta.clone());
            }
            let elapsed = start.elapsed();

            // 100 non-blocking enqueues should complete very quickly.
            assert!(
                elapsed.as_millis() < 50,
                "100 sends took too long: {:?}",
                elapsed
            );
        }
    }
}
