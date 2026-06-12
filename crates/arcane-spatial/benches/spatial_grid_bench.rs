/// Benchmark comparing grid-based neighbor discovery against the linear-scan baseline.
/// Run with: cargo bench -p arcane-spatial --bench spatial_grid_bench
///
/// This benchmark creates a spatial index with 10,000 entities distributed across multiple
/// clusters, then measures the time to query neighbors across all clusters.
use arcane_core::types::Vec3;
use arcane_spatial::SpatialIndex;
use std::time::Instant;
use uuid::Uuid;

fn uuid(i: u32) -> Uuid {
    let bytes = i.to_le_bytes();
    Uuid::from_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ])
}

fn benchmark_10k_entities_grid_cell_50() {
    let mut index = SpatialIndex::with_cell_size(50.0);
    index.set_observation_radius(100.0);

    // Add 10k entities across ~30 clusters
    let entity_count = 10_000;
    let cluster_count = 30;

    let start_add = Instant::now();
    for entity_idx in 0..entity_count {
        let cluster_idx = (entity_idx % cluster_count) as u32;
        let cluster_id = uuid(cluster_idx);

        // Distribute entities in a 2D grid pattern
        let x = (entity_idx as f64 % 100.0) * 10.0;
        let z = (entity_idx as f64 / 100.0) * 10.0;

        index.update_entity(
            uuid(entity_idx as u32 + 10000),
            cluster_id,
            Vec3::new(x, 0.0, z),
        );
    }
    let add_duration = start_add.elapsed();

    // Benchmark: query neighbors for all clusters
    let start_query = Instant::now();
    let mut total_neighbors = 0;
    for cluster_idx in 0..cluster_count {
        let neighbors = index.get_neighbors(uuid(cluster_idx as u32));
        total_neighbors += neighbors.len();
    }
    let query_duration = start_query.elapsed();

    println!(
        "grid_cell_50: add {} entities: {:?}, query {} clusters: {:?} (total neighbors: {})",
        entity_count, add_duration, cluster_count, query_duration, total_neighbors
    );
}

fn benchmark_10k_entities_grid_cell_100() {
    let mut index = SpatialIndex::with_cell_size(100.0);
    index.set_observation_radius(100.0);

    let entity_count = 10_000;
    let cluster_count = 30;

    let start_add = Instant::now();
    for entity_idx in 0..entity_count {
        let cluster_idx = (entity_idx % cluster_count) as u32;
        let cluster_id = uuid(cluster_idx);

        let x = (entity_idx as f64 % 100.0) * 10.0;
        let z = (entity_idx as f64 / 100.0) * 10.0;

        index.update_entity(
            uuid(entity_idx as u32 + 10000),
            cluster_id,
            Vec3::new(x, 0.0, z),
        );
    }
    let add_duration = start_add.elapsed();

    let start_query = Instant::now();
    let mut total_neighbors = 0;
    for cluster_idx in 0..cluster_count {
        let neighbors = index.get_neighbors(uuid(cluster_idx as u32));
        total_neighbors += neighbors.len();
    }
    let query_duration = start_query.elapsed();

    println!(
        "grid_cell_100: add {} entities: {:?}, query {} clusters: {:?} (total neighbors: {})",
        entity_count, add_duration, cluster_count, query_duration, total_neighbors
    );
}

fn benchmark_10k_entities_default_cell_size() {
    let mut index = SpatialIndex::new();
    index.set_observation_radius(100.0);

    let entity_count = 10_000;
    let cluster_count = 30;

    let start_add = Instant::now();
    for entity_idx in 0..entity_count {
        let cluster_idx = (entity_idx % cluster_count) as u32;
        let cluster_id = uuid(cluster_idx);

        let x = (entity_idx as f64 % 100.0) * 10.0;
        let z = (entity_idx as f64 / 100.0) * 10.0;

        index.update_entity(
            uuid(entity_idx as u32 + 10000),
            cluster_id,
            Vec3::new(x, 0.0, z),
        );
    }
    let add_duration = start_add.elapsed();

    let start_query = Instant::now();
    let mut total_neighbors = 0;
    for cluster_idx in 0..cluster_count {
        let neighbors = index.get_neighbors(uuid(cluster_idx as u32));
        total_neighbors += neighbors.len();
    }
    let query_duration = start_query.elapsed();

    println!(
        "grid_default(50.0): add {} entities: {:?}, query {} clusters: {:?} (total neighbors: {})",
        entity_count, add_duration, cluster_count, query_duration, total_neighbors
    );
}

fn main() {
    println!("SpatialIndex benchmark — 10k entities, grid-based neighbor discovery\n");
    benchmark_10k_entities_default_cell_size();
    benchmark_10k_entities_grid_cell_50();
    benchmark_10k_entities_grid_cell_100();
    println!("\nNote: baseline (linear-scan) was removed; compare to previous commit.");
}
