//! Tests for ClusterServer (IN-02). Define expected behavior; implementation must satisfy these.

use std::sync::Arc;

use arcane_infra::{ClusterServer, ClusterSimulation, ReplicationChannelManager};
use uuid::Uuid;

struct NudgePositiveX;

impl ClusterSimulation for NudgePositiveX {
    fn on_tick(&self, ctx: &mut arcane_infra::ClusterTickContext<'_>) {
        for e in ctx.entities.values_mut() {
            e.position.x += 10.0 * ctx.dt_seconds;
        }
    }
}

#[test]
fn new_holds_cluster_id() {
    let id = Uuid::new_v4();
    let server = ClusterServer::new(id);
    assert_eq!(server.cluster_id(), id);
}

#[test]
fn current_tick_starts_at_zero_after_new() {
    let server = ClusterServer::new(Uuid::new_v4());
    let tick = server.current_tick();
    assert_eq!(tick, 0, "tick should be 0 before run");
}

#[test]
fn tick_increments_tick_and_seq() {
    let server = ClusterServer::new(Uuid::new_v4());
    assert_eq!(server.current_tick(), 0);
    assert_eq!(server.current_seq(), 0);
    let _ = server.tick();
    assert_eq!(server.current_tick(), 1);
    assert_eq!(server.current_seq(), 1);
    let _ = server.tick();
    let _ = server.tick();
    assert_eq!(server.current_tick(), 3);
    assert_eq!(server.current_seq(), 3);
}

#[test]
fn tick_with_replication_and_neighbors_sends_delta() {
    let cluster_id = Uuid::new_v4();
    let server = ClusterServer::new(cluster_id);
    let mgr = ReplicationChannelManager::new(cluster_id);
    mgr.set_neighbors(vec![Uuid::new_v4()]);
    server.set_replication(Arc::new(mgr));
    let _ = server.tick();
    let _ = server.tick();
    assert_eq!(server.current_tick(), 2);
    assert_eq!(server.current_seq(), 2);
}

#[test]
fn tick_with_replication_zero_neighbors_no_panic() {
    let cluster_id = Uuid::new_v4();
    let server = ClusterServer::new(cluster_id);
    let mgr = ReplicationChannelManager::new(cluster_id);
    assert_eq!(mgr.channel_count(), 0);
    server.set_replication(Arc::new(mgr));
    let _ = server.tick();
    let _ = server.tick();
    assert_eq!(server.current_tick(), 2);
}

#[test]
fn tick_returns_delta_with_entities() {
    use arcane_core::replication_channel::EntityStateEntry;
    use arcane_core::Vec3;

    let cluster_id = Uuid::new_v4();
    let server = ClusterServer::new(cluster_id);
    let entity_id = Uuid::new_v4();
    server.add_entity(EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));
    let delta = server.tick();
    assert_eq!(delta.source_cluster_id, cluster_id);
    assert_eq!(delta.tick, 1);
    assert_eq!(delta.updated.len(), 1);
    assert_eq!(delta.updated[0].entity_id, entity_id);
    assert_eq!(delta.updated[0].position.x, 1.0);
    assert_eq!(delta.updated[0].position.y, 2.0);
    assert_eq!(delta.updated[0].position.z, 3.0);
    assert!(delta.removed.is_empty());
}

#[test]
fn simulate_before_tick_runs_before_delta_and_sees_upcoming_tick() {
    use arcane_core::replication_channel::EntityStateEntry;
    use arcane_core::Vec3;

    struct RecordTick;
    impl ClusterSimulation for RecordTick {
        fn on_tick(&self, ctx: &mut arcane_infra::ClusterTickContext<'_>) {
            assert_eq!(ctx.tick, 1, "first frame should use upcoming tick 1");
            assert!((ctx.dt_seconds - 0.05).abs() < 1e-9);
            let _ = ctx
                .entities
                .get_mut(&Uuid::nil())
                .expect("entity present")
                .position
                .x;
        }
    }

    let cluster_id = Uuid::new_v4();
    let server = ClusterServer::new(cluster_id);
    server.add_entity(EntityStateEntry::new(
        Uuid::nil(),
        cluster_id,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));
    server.simulate_before_tick(0.05, 1, Some(&RecordTick));
    let delta = server.tick();
    assert_eq!(delta.tick, 1);
}

#[test]
fn simulate_before_tick_can_mutate_positions() {
    use arcane_core::replication_channel::EntityStateEntry;
    use arcane_core::Vec3;

    let cluster_id = Uuid::new_v4();
    let server = ClusterServer::new(cluster_id);
    let entity_id = Uuid::new_v4();
    server.add_entity(EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
    ));
    server.simulate_before_tick(1.0, 1, Some(&NudgePositiveX));
    let delta = server.tick();
    assert_eq!(delta.updated[0].position.x, 10.0);
}

#[test]
fn simulate_before_tick_pending_removals_end_up_in_delta_removed() {
    use arcane_core::replication_channel::EntityStateEntry;
    use arcane_core::Vec3;

    struct RemoveAll;
    impl ClusterSimulation for RemoveAll {
        fn on_tick(&self, ctx: &mut arcane_infra::ClusterTickContext<'_>) {
            for id in ctx.entities.keys().copied().collect::<Vec<_>>() {
                ctx.pending_removals.push(id);
            }
        }
    }

    let cluster_id = Uuid::new_v4();
    let server = ClusterServer::new(cluster_id);
    let entity_id = Uuid::new_v4();
    server.add_entity(EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));
    server.simulate_before_tick(0.05, 1, Some(&RemoveAll));
    let delta = server.tick();
    assert!(delta.updated.is_empty());
    assert_eq!(delta.removed, vec![entity_id]);
}

#[test]
fn remove_entity_appears_in_next_delta_removed() {
    use arcane_core::replication_channel::EntityStateEntry;
    use arcane_core::Vec3;

    let cluster_id = Uuid::new_v4();
    let server = ClusterServer::new(cluster_id);
    let entity_id = Uuid::new_v4();
    server.add_entity(EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));
    let _ = server.tick();
    server.remove_entity(entity_id);
    let delta = server.tick();
    assert!(delta.updated.is_empty());
    assert_eq!(delta.removed.len(), 1);
    assert_eq!(delta.removed[0], entity_id);
}
