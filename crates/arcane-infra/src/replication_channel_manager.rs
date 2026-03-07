//! ReplicationChannelManager (IN-06) — manage IReplicationChannel per neighbor.
//! Uses a single Redis pub/sub broadcast (one topic per cluster); neighbors subscribe to our topic.

use std::sync::Mutex;

use arcane_core::replication_channel::{CloseReason, EntityStateDelta, IReplicationChannel};
use uuid::Uuid;

use crate::redis_channel::RedisReplicationChannel;

/// Runs on each ClusterServer. Opens/closes channels from topology; send_to_neighbors, on_receive.
pub struct ReplicationChannelManager {
    cluster_id: Uuid,
    inner: Mutex<ManagerState>,
}

struct ManagerState {
    /// Single broadcast channel (publishes to arcane:replication:{cluster_id}). None until start() with redis.
    channel: Option<RedisReplicationChannel>,
    /// Current neighbor cluster IDs (from topology). channel_count() = this length.
    neighbors: Vec<Uuid>,
}

impl ReplicationChannelManager {
    pub fn new(cluster_id: Uuid) -> Self {
        Self {
            cluster_id,
            inner: Mutex::new(ManagerState {
                channel: None,
                neighbors: Vec::new(),
            }),
        }
    }

    /// Start: connect to Redis and create the broadcast channel. Call set_neighbors() to set recipient count.
    pub fn start(&self, redis_url: &str) -> Result<(), String> {
        let client = redis::Client::open(redis_url).map_err(|e| e.to_string())?;
        let conn = client.get_connection().map_err(|e| e.to_string())?;
        let channel = RedisReplicationChannel::new(self.cluster_id, conn);
        let mut state = self.inner.lock().map_err(|_| "lock poisoned")?;
        state.channel = Some(channel);
        Ok(())
    }

    /// Stop: close the broadcast channel (SHUTDOWN), clear neighbors.
    pub fn stop(&self) {
        let mut state = self.inner.lock().expect("lock");
        if let Some(ref ch) = state.channel {
            ch.close(CloseReason::Shutdown);
        }
        state.channel = None;
        state.neighbors.clear();
    }

    /// Set current neighbor cluster IDs (from topology). Replaces previous list.
    pub fn set_neighbors(&self, neighbor_ids: Vec<Uuid>) {
        let mut state = self.inner.lock().expect("lock");
        state.neighbors = neighbor_ids;
    }

    /// Broadcast delta to all current neighbor channels. Non-blocking; no-op if not started or no neighbors.
    pub fn send_to_neighbors(&self, delta: EntityStateDelta) {
        let state = self.inner.lock().expect("lock");
        if let Some(ref ch) = state.channel {
            if !state.neighbors.is_empty() {
                ch.send(delta);
            }
        }
    }

    /// Number of neighbor channels we are broadcasting to (length of set_neighbors).
    pub fn channel_count(&self) -> usize {
        let state = self.inner.lock().expect("lock");
        state.neighbors.len()
    }

    pub fn cluster_id(&self) -> Uuid {
        self.cluster_id
    }
}
