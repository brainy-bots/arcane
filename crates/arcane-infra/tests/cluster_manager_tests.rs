//! Tests for ClusterManager (IN-01). Define expected behavior; implementation must satisfy these.

use arcane_core::Vec3;
use arcane_infra::ClusterManager;
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

#[test]
fn with_defaults_creates_manager() {
    let _manager = ClusterManager::with_defaults();
}

#[test]
fn active_cluster_count_initially_zero() {
    let manager = ClusterManager::with_defaults();
    let count = manager.active_cluster_count();
    assert_eq!(count, 0, "no clusters before run or any assignment");
}

#[test]
fn get_neighbors_for_cluster_returns_neighbors_from_spatial_index() {
    let mut manager = ClusterManager::with_defaults();
    manager.set_observation_radius(100.0);
    // Cluster A: entities at (0,0,0) and (100,0,0) -> centroid ~(50,0,0), spread ~50
    manager.update_entity(uuid(10), uuid(1), Vec3::new(0.0, 0.0, 0.0));
    manager.update_entity(uuid(11), uuid(1), Vec3::new(100.0, 0.0, 0.0));
    // Cluster B: entities at (200,0,0) and (300,0,0) -> centroid (250,0,0), spread ~50; distance A–B = 200
    manager.update_entity(uuid(20), uuid(2), Vec3::new(200.0, 0.0, 0.0));
    manager.update_entity(uuid(21), uuid(2), Vec3::new(300.0, 0.0, 0.0));
    let neighbors_a = manager.get_neighbors_for_cluster(uuid(1));
    let neighbors_b = manager.get_neighbors_for_cluster(uuid(2));
    assert!(
        neighbors_a.contains(&uuid(2)),
        "A's neighbors should include B"
    );
    assert!(
        neighbors_b.contains(&uuid(1)),
        "B's neighbors should include A"
    );
}
