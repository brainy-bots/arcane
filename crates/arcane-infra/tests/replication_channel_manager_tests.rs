//! Tests for ReplicationChannelManager (IN-06). Define expected behavior.

use arcane_core::replication_channel::{EntityStateEntry, EntityStateDelta};
use arcane_core::Vec3;
use arcane_infra::ReplicationChannelManager;
use uuid::Uuid;

#[test]
fn new_holds_cluster_id() {
    let id = Uuid::new_v4();
    let mgr = ReplicationChannelManager::new(id);
    assert_eq!(mgr.cluster_id(), id);
}

#[test]
fn channel_count_zero_before_start() {
    let mgr = ReplicationChannelManager::new(Uuid::new_v4());
    let n = mgr.channel_count();
    assert_eq!(n, 0, "no channels before start or topology");
}

#[test]
fn set_neighbors_updates_channel_count() {
    let mgr = ReplicationChannelManager::new(Uuid::new_v4());
    assert_eq!(mgr.channel_count(), 0);
    mgr.set_neighbors(vec![Uuid::new_v4()]);
    assert_eq!(mgr.channel_count(), 1);
    mgr.set_neighbors(vec![Uuid::new_v4(), Uuid::new_v4()]);
    assert_eq!(mgr.channel_count(), 2);
    mgr.set_neighbors(vec![]);
    assert_eq!(mgr.channel_count(), 0);
}

#[test]
fn send_to_neighbors_no_panic_before_start() {
    let mgr = ReplicationChannelManager::new(Uuid::new_v4());
    let delta = EntityStateDelta {
        source_cluster_id: mgr.cluster_id(),
        seq: 0,
        tick: 0,
        timestamp: 0.0,
        updated: vec![EntityStateEntry {
            entity_id: Uuid::new_v4(),
            cluster_id: mgr.cluster_id(),
            position: Vec3::new(0.0, 0.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, 0.0),
        }],
        removed: vec![],
    };
    mgr.send_to_neighbors(delta);
}

/// When Redis is up: start, set_neighbors, send_to_neighbors and channel_count work.
#[test]
fn start_set_neighbors_send_when_redis_up() {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let mgr = ReplicationChannelManager::new(Uuid::new_v4());
    if mgr.start(&url).is_err() {
        return; // Redis not available
    }
    mgr.set_neighbors(vec![Uuid::new_v4()]);
    assert!(mgr.channel_count() >= 1);
    let delta = EntityStateDelta {
        source_cluster_id: mgr.cluster_id(),
        seq: 1,
        tick: 1,
        timestamp: 1.0,
        updated: vec![],
        removed: vec![],
    };
    mgr.send_to_neighbors(delta);
    mgr.stop();
    assert_eq!(mgr.channel_count(), 0);
}
