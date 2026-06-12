//! Micro-benchmark for SpatialIndex update + neighbor-query cost (issue #169).
//! Run with: cargo bench -p arcane-spatial --bench spatial_grid_bench
//!
//! 10,000 entities across 30 clusters on a 1 km x 1 km plane; reports wall time
//! for the full insert pass, a re-update pass (cache-dirtying writes), and a
//! neighbor query for every cluster.

use arcane_core::types::Vec3;
use arcane_spatial::SpatialIndex;
use std::time::Instant;
use uuid::Uuid;

fn uuid(i: u32) -> Uuid {
    let b = i.to_le_bytes();
    Uuid::from_bytes([b[0], b[1], b[2], b[3], 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn run(cell_size: f64) {
    let mut index = SpatialIndex::with_config(cell_size, 1.0);
    index.set_observation_radius(100.0);

    let entity_count: u32 = 10_000;
    let cluster_count: u32 = 30;

    let pos = |i: u32| Vec3::new(f64::from(i % 100) * 10.0, 0.0, f64::from(i / 100) * 10.0);

    let t = Instant::now();
    for i in 0..entity_count {
        index.update_entity(uuid(i + 10_000), uuid(i % cluster_count), pos(i));
    }
    let insert = t.elapsed();

    let t = Instant::now();
    for i in 0..entity_count {
        index.update_entity(uuid(i + 10_000), uuid(i % cluster_count), pos(i + 1));
    }
    let update = t.elapsed();

    let t = Instant::now();
    let mut total = 0usize;
    for c in 0..cluster_count {
        total += index.get_neighbors(uuid(c)).len();
    }
    let query = t.elapsed();

    println!(
        "cell={cell_size}: insert 10k {insert:?}, re-update 10k {update:?}, query {cluster_count} clusters {query:?} (neighbor pairs: {total})"
    );
}

fn main() {
    run(50.0);
    run(100.0);
    run(200.0);
}
