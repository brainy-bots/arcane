//! Tests for SpatialIndex (IN-03). Define expected behavior; implementation must satisfy these.

use arcane_core::types::Vec3;
use arcane_spatial::SpatialIndex;
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

#[test]
fn new_index_has_no_clusters() {
    let index = SpatialIndex::new();
    let snapshot = index.snapshot_for_view();
    assert!(snapshot.is_empty(), "new index should have no clusters");
}

#[test]
fn update_entity_single_entity_returns_geometry() {
    let mut index = SpatialIndex::new();
    let cluster_a = uuid(1);
    let entity_1 = uuid(10);
    let pos = Vec3::new(100.0, 0.0, 200.0);

    index.update_entity(entity_1, cluster_a, pos);

    let geom = index
        .get_cluster_geometry(cluster_a)
        .expect("cluster should exist");
    assert_eq!(geom.cluster_id, cluster_a);
    assert_eq!(geom.entity_count, 1);
    assert_eq!(geom.centroid.x, 100.0);
    assert_eq!(geom.centroid.z, 200.0);
    assert_eq!(geom.spread_radius, 0.0, "single entity has zero spread");
}

#[test]
fn update_entity_two_entities_same_cluster_centroid_and_spread() {
    let mut index = SpatialIndex::new();
    let cluster_a = uuid(1);
    let pos1 = Vec3::new(0.0, 0.0, 0.0);
    let pos2 = Vec3::new(10.0, 0.0, 0.0);

    index.update_entity(uuid(10), cluster_a, pos1);
    index.update_entity(uuid(11), cluster_a, pos2);

    let geom = index
        .get_cluster_geometry(cluster_a)
        .expect("cluster should exist");
    assert_eq!(geom.entity_count, 2);
    assert_eq!(geom.centroid.x, 5.0, "centroid x is midpoint");
    assert_eq!(geom.centroid.z, 0.0);
    assert!(
        geom.spread_radius > 0.0,
        "two entities have positive spread"
    );
}

#[test]
fn remove_entity_empty_cluster_returns_none() {
    let mut index = SpatialIndex::new();
    let cluster_a = uuid(1);
    let entity_1 = uuid(10);
    index.update_entity(entity_1, cluster_a, Vec3::new(0.0, 0.0, 0.0));

    index.remove_entity(entity_1, cluster_a);

    assert!(
        index.get_cluster_geometry(cluster_a).is_none(),
        "empty cluster should be absent"
    );
}

#[test]
fn remove_entity_one_of_two_updates_geometry() {
    let mut index = SpatialIndex::new();
    let cluster_a = uuid(1);
    index.update_entity(uuid(10), cluster_a, Vec3::new(0.0, 0.0, 0.0));
    index.update_entity(uuid(11), cluster_a, Vec3::new(10.0, 0.0, 0.0));

    index.remove_entity(uuid(11), cluster_a);

    let geom = index
        .get_cluster_geometry(cluster_a)
        .expect("cluster should still exist");
    assert_eq!(geom.entity_count, 1);
    assert_eq!(geom.centroid.x, 0.0);
    assert_eq!(geom.spread_radius, 0.0);
}

#[test]
fn get_neighbors_two_nearby_clusters_see_each_other() {
    let mut index = SpatialIndex::new();
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);
    // Place clusters close: A at (0,0,0), B at (50,0,0). With observation_radius large enough they overlap.
    index.update_entity(uuid(10), cluster_a, Vec3::new(0.0, 0.0, 0.0));
    index.update_entity(uuid(20), cluster_b, Vec3::new(50.0, 0.0, 0.0));
    index.set_observation_radius(100.0); // so effective areas overlap

    let neighbors_a = index.get_neighbors(cluster_a);
    let neighbors_b = index.get_neighbors(cluster_b);

    assert!(
        neighbors_a.contains(&cluster_b),
        "A's neighbors should include B"
    );
    assert!(
        neighbors_b.contains(&cluster_a),
        "B's neighbors should include A"
    );
}

#[test]
fn get_neighbors_far_clusters_not_neighbors() {
    let mut index = SpatialIndex::new();
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);
    index.update_entity(uuid(10), cluster_a, Vec3::new(0.0, 0.0, 0.0));
    index.update_entity(uuid(20), cluster_b, Vec3::new(1000.0, 0.0, 1000.0));
    index.set_observation_radius(50.0);

    let neighbors_a = index.get_neighbors(cluster_a);

    assert!(
        !neighbors_a.contains(&cluster_b),
        "distant B should not be neighbor of A"
    );
}

#[test]
fn snapshot_for_view_returns_all_clusters() {
    let mut index = SpatialIndex::new();
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);
    index.update_entity(uuid(10), cluster_a, Vec3::new(0.0, 0.0, 0.0));
    index.update_entity(uuid(20), cluster_b, Vec3::new(1.0, 0.0, 1.0));

    let snapshot = index.snapshot_for_view();

    assert_eq!(snapshot.len(), 2);
    let ids: Vec<Uuid> = snapshot.iter().map(|g| g.cluster_id).collect();
    assert!(ids.contains(&cluster_a));
    assert!(ids.contains(&cluster_b));
}

#[test]
fn entity_move_between_clusters_updates_both() {
    let mut index = SpatialIndex::new();
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);
    let entity_1 = uuid(10);
    index.update_entity(entity_1, cluster_a, Vec3::new(0.0, 0.0, 0.0));
    index.update_entity(uuid(11), cluster_a, Vec3::new(2.0, 0.0, 0.0)); // two in A

    index.update_entity(entity_1, cluster_b, Vec3::new(100.0, 0.0, 100.0)); // move entity_1 to B

    let geom_a = index
        .get_cluster_geometry(cluster_a)
        .expect("A should exist");
    let geom_b = index
        .get_cluster_geometry(cluster_b)
        .expect("B should exist");
    assert_eq!(geom_a.entity_count, 1, "A should have one entity left");
    assert_eq!(geom_b.entity_count, 1, "B should have the moved entity");
    assert_eq!(geom_a.centroid.x, 2.0);
    assert_eq!(geom_b.centroid.x, 100.0);
}

#[test]
fn get_clusters_in_region_returns_matching_clusters() {
    let mut index = SpatialIndex::new();
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);
    let cluster_c = uuid(3);
    // A near origin, B far away, C at (50,0,50)
    index.update_entity(uuid(10), cluster_a, Vec3::new(0.0, 0.0, 0.0));
    index.update_entity(uuid(20), cluster_b, Vec3::new(1000.0, 0.0, 1000.0));
    index.update_entity(uuid(30), cluster_c, Vec3::new(50.0, 0.0, 50.0));

    let in_region = index.get_clusters_in_region((0.0, 0.0), 100.0);

    assert!(in_region.contains(&cluster_a), "A is within radius");
    assert!(in_region.contains(&cluster_c), "C is within radius");
    assert!(!in_region.contains(&cluster_b), "B is far away");
}

#[test]
fn get_clusters_in_region_empty_index_returns_empty() {
    let index = SpatialIndex::new();
    let result = index.get_clusters_in_region((0.0, 0.0), 100.0);
    assert!(result.is_empty());
}
