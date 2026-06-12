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

// --- issue #169: sparse-hash internals must match brute-force semantics ---

/// Brute-force reference: weighted 3D effective-area overlap over snapshot geometry.
fn brute_force_neighbors(
    index: &SpatialIndex,
    cluster_id: Uuid,
    observation_radius: f64,
    y_weight: f64,
) -> Vec<Uuid> {
    let snapshot = index.snapshot_for_view();
    let me = match snapshot.iter().find(|g| g.cluster_id == cluster_id) {
        Some(g) => g.clone(),
        None => return vec![],
    };
    // Weighted spread must be recomputed from entities (snapshot spread is world-space).
    let weighted_spread = |cid: Uuid, centroid: &Vec3| -> f64 {
        index
            .snapshot_entities()
            .iter()
            .filter(|(_, c, _)| *c == cid)
            .map(|(_, _, p)| {
                let dx = p.x - centroid.x;
                let dy = (p.y - centroid.y) * y_weight;
                let dz = p.z - centroid.z;
                (dx * dx + dy * dy + dz * dz).sqrt()
            })
            .fold(0.0_f64, f64::max)
    };
    let my_spread = weighted_spread(cluster_id, &me.centroid);
    let mut out: Vec<Uuid> = snapshot
        .iter()
        .filter(|g| g.cluster_id != cluster_id)
        .filter(|g| {
            let dx = me.centroid.x - g.centroid.x;
            let dy = (me.centroid.y - g.centroid.y) * y_weight;
            let dz = me.centroid.z - g.centroid.z;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            dist <= my_spread
                + observation_radius
                + weighted_spread(g.cluster_id, &g.centroid)
                + observation_radius
        })
        .map(|g| g.cluster_id)
        .collect();
    out.sort();
    out
}

#[test]
fn grid_neighbors_match_brute_force_pseudorandom() {
    // Deterministic pseudo-random layout: 12 clusters x 8 entities over a 1km box.
    let mut index = SpatialIndex::new();
    index.set_observation_radius(30.0);
    let mut seed: u64 = 0x5eed;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as f64 / u32::MAX as f64) * 1000.0 - 500.0
    };
    for c in 0..12u8 {
        for e in 0..8u8 {
            index.update_entity(
                Uuid::from_bytes([200 + c, e, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
                uuid(c + 1),
                Vec3::new(next(), next() * 0.1, next()),
            );
        }
    }
    for c in 0..12u8 {
        let id = uuid(c + 1);
        assert_eq!(
            index.get_neighbors(id),
            brute_force_neighbors(&index, id, 30.0, 1.0),
            "grid result must match brute force for cluster {c}"
        );
    }
}

#[test]
fn adversarial_wide_cluster_far_edge_update_still_found() {
    // Review finding on the first grid attempt: a cluster whose last-updated entity
    // sits at its far edge must still be found by neighbors it overlaps.
    let mut index = SpatialIndex::new();
    index.set_observation_radius(10.0);
    let wide = uuid(1);
    let probe = uuid(2);

    // Wide cluster: centroid near x=250, spread ~250 (entities at 0 and 500).
    index.update_entity(uuid(10), wide, Vec3::new(0.0, 0.0, 0.0));
    index.update_entity(uuid(11), wide, Vec3::new(500.0, 0.0, 0.0));
    // Probe cluster just inside overlap range on the LEFT, far from the last-updated entity.
    index.update_entity(uuid(20), probe, Vec3::new(-60.0, 0.0, 0.0));

    // Overlap: dist(250, -60) = 310 <= (250 + 10) + (0 + 10) = 270? No — push probe closer.
    // dist must be <= 270, so put probe at -15: dist = 265.
    index.update_entity(uuid(20), probe, Vec3::new(-15.0, 0.0, 0.0));

    assert!(
        index.get_neighbors(wide).contains(&probe),
        "wide cluster must see the probe regardless of which edge updated last"
    );
    assert!(
        index.get_neighbors(probe).contains(&wide),
        "neighbor relation must be symmetric"
    );
}

#[test]
fn layered_world_vertical_weight_separates_floors() {
    // Two clusters at identical x/z, 200 apart vertically.
    let build = |y_weight: f64| {
        let mut index = SpatialIndex::with_config(50.0, y_weight);
        index.set_observation_radius(40.0);
        index.update_entity(uuid(10), uuid(1), Vec3::new(0.0, 0.0, 0.0));
        index.update_entity(uuid(20), uuid(2), Vec3::new(0.0, 200.0, 0.0));
        index
    };

    // y_weight 0 → legacy 2D: vertical distance ignored, they are neighbors.
    let flat = build(0.0);
    assert!(
        flat.get_neighbors(uuid(1)).contains(&uuid(2)),
        "y_weight=0 ignores vertical separation (legacy 2D)"
    );

    // y_weight 1 → 3D sphere: 200 > 80 effective range, not neighbors.
    let sphere = build(1.0);
    assert!(
        sphere.get_neighbors(uuid(1)).is_empty(),
        "y_weight=1 separates floors 200 units apart"
    );
}

#[test]
fn cluster_migration_rebuckets_and_empties_cleanly() {
    let mut index = SpatialIndex::new();
    index.set_observation_radius(10.0);
    // Entity starts in cluster A, moves far away into cluster B.
    index.update_entity(uuid(10), uuid(1), Vec3::new(0.0, 0.0, 0.0));
    index.update_entity(uuid(10), uuid(2), Vec3::new(900.0, 0.0, 900.0));

    assert!(
        index.get_cluster_geometry(uuid(1)).is_none(),
        "emptied cluster disappears from the index"
    );
    let geom = index
        .get_cluster_geometry(uuid(2))
        .expect("cluster B exists");
    assert_eq!(geom.entity_count, 1);
    assert_eq!(geom.centroid.x, 900.0);
    assert!(
        index.get_neighbors(uuid(2)).is_empty(),
        "sole cluster has no neighbors"
    );
}
