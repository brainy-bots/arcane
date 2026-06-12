//! SpatialIndex — per-cluster geometry and neighbor discovery.
//!
//! API from IN-03: update_entity, remove_entity, set_observation_radius,
//! get_cluster_geometry, get_neighbors, get_clusters_in_region, snapshot_for_view.
//!
//! Optimized with a spatial grid to avoid O(entities × clusters) neighbor queries.
//! Internally uses per-cluster entity buckets and a grid over cluster centroids.

use arcane_core::types::{ClusterGeometry, Vec3};
use std::cell::RefCell;
use std::collections::HashMap;
use uuid::Uuid;

/// Per-cluster entity bucket: stores entity data with cached geometry.
struct ClusterBucket {
    entities: HashMap<Uuid, Vec3>,
    position_sum: Vec3,
    cached_geometry: RefCell<Option<ClusterGeometry>>,
}

impl ClusterBucket {
    fn new() -> Self {
        Self {
            entities: HashMap::new(),
            position_sum: Vec3::new(0.0, 0.0, 0.0),
            cached_geometry: RefCell::new(None),
        }
    }

    fn add_entity(&mut self, entity_id: Uuid, position: Vec3) {
        self.entities.insert(entity_id, position);
        self.position_sum.x += position.x;
        self.position_sum.y += position.y;
        self.position_sum.z += position.z;
        *self.cached_geometry.borrow_mut() = None; // Mark cache invalid
    }

    fn remove_entity(&mut self, entity_id: Uuid) -> Option<Vec3> {
        if let Some(position) = self.entities.remove(&entity_id) {
            self.position_sum.x -= position.x;
            self.position_sum.y -= position.y;
            self.position_sum.z -= position.z;
            *self.cached_geometry.borrow_mut() = None; // Mark cache invalid
            Some(position)
        } else {
            None
        }
    }

    fn compute_and_cache_geometry(&mut self, cluster_id: Uuid) -> Option<ClusterGeometry> {
        let count = self.entities.len();
        if count == 0 {
            *self.cached_geometry.borrow_mut() = None;
            return None;
        }

        let n = count as f64;
        let centroid = Vec3 {
            x: self.position_sum.x / n,
            y: self.position_sum.y / n,
            z: self.position_sum.z / n,
        };
        let spread_radius = self
            .entities
            .values()
            .map(|p| p.distance_sq_to(&centroid).sqrt())
            .fold(0.0_f64, f64::max);

        let geom = ClusterGeometry {
            cluster_id,
            centroid,
            spread_radius,
            entity_count: count as u32,
        };
        *self.cached_geometry.borrow_mut() = Some(geom.clone());
        Some(geom)
    }

    fn get_cached_geometry(&self) -> Option<ClusterGeometry> {
        self.cached_geometry.borrow().clone()
    }
}

/// 2D spatial grid cell key.
#[derive(Eq, PartialEq, Hash, Clone, Copy)]
struct GridCell(i32, i32);

impl GridCell {
    fn from_position(pos: Vec3, cell_size: f64) -> Self {
        let cell_x = (pos.x / cell_size).floor() as i32;
        let cell_z = (pos.z / cell_size).floor() as i32;
        GridCell(cell_x, cell_z)
    }

    /// All grid cells within a given search radius from this cell.
    fn cells_within_radius(self, search_radius: f64, cell_size: f64) -> Vec<GridCell> {
        let cell_count = (search_radius / cell_size).ceil() as i32 + 1;
        let mut result = Vec::new();
        for dx in -cell_count..=cell_count {
            for dz in -cell_count..=cell_count {
                result.push(GridCell(self.0 + dx, self.1 + dz));
            }
        }
        result
    }
}

/// 2D coarse spatial index over cluster entities. Caller (e.g. ArcaneManager) feeds
/// entity positions via update_entity / remove_entity; index answers geometry and neighbor queries.
pub struct SpatialIndex {
    observation_radius: f64,
    grid_cell_size: f64,
    /// cluster_id -> bucket
    clusters: HashMap<Uuid, ClusterBucket>,
    /// entity_id -> cluster_id (reverse map for O(1) cluster lookup on update/remove)
    entity_to_cluster: HashMap<Uuid, Uuid>,
    /// grid_cell -> set of cluster_ids in that cell
    grid: HashMap<GridCell, std::collections::HashSet<Uuid>>,
    /// cluster_id -> current grid_cell (reverse map for O(1) cell lookup on move)
    cluster_to_cell: HashMap<Uuid, GridCell>,
}

impl SpatialIndex {
    /// Create a new index with default cell size (50.0). Call set_observation_radius before get_neighbors.
    pub fn new() -> Self {
        Self::with_cell_size(50.0)
    }

    /// Create a new index with a specified grid cell size.
    pub fn with_cell_size(cell_size: f64) -> Self {
        Self {
            observation_radius: 0.0,
            grid_cell_size: cell_size.max(1.0),
            clusters: HashMap::new(),
            entity_to_cluster: HashMap::new(),
            grid: HashMap::new(),
            cluster_to_cell: HashMap::new(),
        }
    }

    /// Register or update an entity's position and cluster. If cluster_id changed, updates both clusters.
    pub fn update_entity(&mut self, entity_id: Uuid, cluster_id: Uuid, position: Vec3) {
        // If entity already exists in a different cluster, remove it from the old cluster.
        if let Some(&old_cluster_id) = self.entity_to_cluster.get(&entity_id) {
            if old_cluster_id != cluster_id {
                if let Some(bucket) = self.clusters.get_mut(&old_cluster_id) {
                    bucket.remove_entity(entity_id);
                    if bucket.entities.is_empty() {
                        self.clusters.remove(&old_cluster_id);
                        // Remove from grid using reverse map (O(1) instead of O(grid cells))
                        if let Some(old_cell) = self.cluster_to_cell.remove(&old_cluster_id) {
                            if let Some(cell_clusters) = self.grid.get_mut(&old_cell) {
                                cell_clusters.remove(&old_cluster_id);
                                if cell_clusters.is_empty() {
                                    self.grid.remove(&old_cell);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Add/update entity in new cluster and update grid position based on centroid
        let old_cell = self.cluster_to_cell.get(&cluster_id).copied();
        {
            let bucket = self
                .clusters
                .entry(cluster_id)
                .or_insert_with(ClusterBucket::new);
            bucket.add_entity(entity_id, position);
            // Compute and cache geometry immediately after modifying bucket
            if let Some(geom) = bucket.compute_and_cache_geometry(cluster_id) {
                let new_cell = GridCell::from_position(geom.centroid, self.grid_cell_size);

                // Move cluster from old cell to new cell if cell changed (O(1) per move)
                if old_cell != Some(new_cell) {
                    if let Some(old) = old_cell {
                        if let Some(cell_clusters) = self.grid.get_mut(&old) {
                            cell_clusters.remove(&cluster_id);
                            if cell_clusters.is_empty() {
                                self.grid.remove(&old);
                            }
                        }
                    }
                    self.grid.entry(new_cell).or_default().insert(cluster_id);
                    self.cluster_to_cell.insert(cluster_id, new_cell);
                }
            }
        }
        self.entity_to_cluster.insert(entity_id, cluster_id);
    }

    /// Remove an entity (despawn or reassignment). Updates that cluster's centroid and spread.
    pub fn remove_entity(&mut self, entity_id: Uuid, _cluster_id: Uuid) {
        if let Some(&cluster_id) = self.entity_to_cluster.get(&entity_id) {
            if let Some(bucket) = self.clusters.get_mut(&cluster_id) {
                bucket.remove_entity(entity_id);
                if bucket.entities.is_empty() {
                    self.clusters.remove(&cluster_id);
                    // Remove empty cluster from grid using reverse map (O(1))
                    if let Some(old_cell) = self.cluster_to_cell.remove(&cluster_id) {
                        if let Some(cell_clusters) = self.grid.get_mut(&old_cell) {
                            cell_clusters.remove(&cluster_id);
                            if cell_clusters.is_empty() {
                                self.grid.remove(&old_cell);
                            }
                        }
                    }
                }
            }
            self.entity_to_cluster.remove(&entity_id);
        }
    }

    /// Set observation radius used for get_neighbors() effective area. Typically from config.
    pub fn set_observation_radius(&mut self, radius: f64) {
        self.observation_radius = radius;
    }

    /// Return centroid, spread_radius, and entity_count for a cluster, or None if not in index.
    pub fn get_cluster_geometry(&self, cluster_id: Uuid) -> Option<ClusterGeometry> {
        self.clusters.get(&cluster_id).and_then(|bucket| {
            // Return cached geometry if available
            if let Some(cached) = bucket.get_cached_geometry() {
                return Some(cached);
            }

            // Compute geometry on-demand if not cached (fallback for clusters not yet updated via update_entity)
            let count = bucket.entities.len();
            if count == 0 {
                return None;
            }

            let n = count as f64;
            let centroid = Vec3 {
                x: bucket.position_sum.x / n,
                y: bucket.position_sum.y / n,
                z: bucket.position_sum.z / n,
            };
            let spread_radius = bucket
                .entities
                .values()
                .map(|p| p.distance_sq_to(&centroid).sqrt())
                .fold(0.0_f64, f64::max);

            let geom = ClusterGeometry {
                cluster_id,
                centroid,
                spread_radius,
                entity_count: count as u32,
            };
            *bucket.cached_geometry.borrow_mut() = Some(geom.clone());
            Some(geom)
        })
    }

    /// Return cluster_ids whose effective area (centroid + spread_radius + observation_radius) overlaps this cluster's.
    pub fn get_neighbors(&self, cluster_id: Uuid) -> Vec<Uuid> {
        // Get geometry of the query cluster
        let geom = match self.get_cluster_geometry(cluster_id) {
            Some(g) => g,
            None => return vec![],
        };

        let effective_self = geom.spread_radius + self.observation_radius;

        // Find grid cells to search
        let query_cell = GridCell::from_position(geom.centroid, self.grid_cell_size);
        // Search radius must include spread of this cluster and max possible spread of others
        let max_spread = self
            .clusters
            .keys()
            .filter_map(|cid| self.get_cluster_geometry(*cid).map(|g| g.spread_radius))
            .fold(0.0_f64, f64::max);
        let search_radius = effective_self + max_spread + self.observation_radius;
        let nearby_cells = query_cell.cells_within_radius(search_radius, self.grid_cell_size);

        // Collect candidate clusters from grid
        let mut candidates = std::collections::HashSet::new();
        for cell in nearby_cells {
            if let Some(cell_clusters) = self.grid.get(&cell) {
                for &candidate_id in cell_clusters {
                    if candidate_id != cluster_id {
                        candidates.insert(candidate_id);
                    }
                }
            }
        }

        // Check which candidates actually overlap
        let mut neighbors = Vec::new();
        for candidate_id in candidates {
            if let Some(other_geom) = self.get_cluster_geometry(candidate_id) {
                let effective_other = other_geom.spread_radius + self.observation_radius;
                let dx = geom.centroid.x - other_geom.centroid.x;
                let dz = geom.centroid.z - other_geom.centroid.z;
                let dist_2d = (dx * dx + dz * dz).sqrt();
                if dist_2d <= effective_self + effective_other {
                    neighbors.push(candidate_id);
                }
            }
        }

        neighbors.sort();
        neighbors
    }

    /// Return cluster_ids that have any entity in the given 2D region (center x/z, radius). Optional API.
    pub fn get_clusters_in_region(&self, center: (f64, f64), radius: f64) -> Vec<Uuid> {
        let (cx, cz) = center;
        let r_sq = radius * radius;
        let mut cluster_ids: Vec<Uuid> = self
            .clusters
            .iter()
            .filter_map(|(cluster_id, bucket)| {
                for pos in bucket.entities.values() {
                    let dx = pos.x - cx;
                    let dz = pos.z - cz;
                    if dx * dx + dz * dz <= r_sq {
                        return Some(*cluster_id);
                    }
                }
                None
            })
            .collect();
        cluster_ids.sort();
        cluster_ids.dedup();
        cluster_ids
    }

    /// Return all entities as (entity_id, cluster_id, position) triples.
    /// Used by ArcaneManager to populate WorldStateView.players.
    pub fn snapshot_entities(&self) -> Vec<(Uuid, Uuid, Vec3)> {
        let mut result = Vec::new();
        for (cluster_id, bucket) in &self.clusters {
            for (&entity_id, &position) in &bucket.entities {
                result.push((entity_id, *cluster_id, position));
            }
        }
        result.sort_by_key(|&(entity_id, _, _)| entity_id);
        result
    }

    /// Snapshot of all clusters for building WorldStateView. Called by ArcaneManager before evaluate().
    pub fn snapshot_for_view(&self) -> Vec<ClusterGeometry> {
        let mut result: Vec<ClusterGeometry> = self
            .clusters
            .keys()
            .filter_map(|cluster_id| self.get_cluster_geometry(*cluster_id))
            .collect();
        result.sort_by_key(|g| g.cluster_id);
        result
    }
}

impl Default for SpatialIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn uuid(i: u8) -> Uuid {
        Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
    }

    /// Brute-force neighbor computation (always checks all clusters).
    /// Used as a reference to verify grid-based optimization.
    fn brute_force_neighbors(
        clusters: &HashMap<Uuid, Vec<Vec3>>,
        observation_radius: f64,
        query_cluster_id: Uuid,
    ) -> Vec<Uuid> {
        let query_positions = match clusters.get(&query_cluster_id) {
            Some(positions) if !positions.is_empty() => positions,
            _ => return vec![],
        };

        let query_n = query_positions.len() as f64;
        let query_sum = query_positions
            .iter()
            .fold(Vec3::new(0.0, 0.0, 0.0), |acc, p| Vec3 {
                x: acc.x + p.x,
                y: acc.y + p.y,
                z: acc.z + p.z,
            });
        let query_centroid = Vec3 {
            x: query_sum.x / query_n,
            y: query_sum.y / query_n,
            z: query_sum.z / query_n,
        };
        let query_spread = query_positions
            .iter()
            .map(|p| p.distance_sq_to(&query_centroid).sqrt())
            .fold(0.0_f64, f64::max);

        let mut neighbors = Vec::new();
        let query_effective = query_spread + observation_radius;

        for (&other_id, other_positions) in clusters {
            if other_id == query_cluster_id || other_positions.is_empty() {
                continue;
            }
            let other_n = other_positions.len() as f64;
            let other_sum = other_positions
                .iter()
                .fold(Vec3::new(0.0, 0.0, 0.0), |acc, p| Vec3 {
                    x: acc.x + p.x,
                    y: acc.y + p.y,
                    z: acc.z + p.z,
                });
            let other_centroid = Vec3 {
                x: other_sum.x / other_n,
                y: other_sum.y / other_n,
                z: other_sum.z / other_n,
            };
            let other_spread = other_positions
                .iter()
                .map(|p| p.distance_sq_to(&other_centroid).sqrt())
                .fold(0.0_f64, f64::max);
            let other_effective = other_spread + observation_radius;

            let dx = query_centroid.x - other_centroid.x;
            let dz = query_centroid.z - other_centroid.z;
            let dist_2d = (dx * dx + dz * dz).sqrt();

            if dist_2d <= query_effective + other_effective {
                neighbors.push(other_id);
            }
        }

        neighbors.sort();
        neighbors
    }

    #[test]
    fn grid_neighbors_matches_brute_force_small_clusters() {
        let mut index = SpatialIndex::with_cell_size(50.0);
        index.set_observation_radius(100.0);

        // Add three clusters at different positions
        let cluster_a = uuid(1);
        let cluster_b = uuid(2);
        let cluster_c = uuid(3);

        // A at origin
        index.update_entity(uuid(10), cluster_a, Vec3::new(0.0, 0.0, 0.0));
        index.update_entity(uuid(11), cluster_a, Vec3::new(10.0, 0.0, 0.0));

        // B nearby (should be neighbor)
        index.update_entity(uuid(20), cluster_b, Vec3::new(80.0, 0.0, 0.0));
        index.update_entity(uuid(21), cluster_b, Vec3::new(90.0, 0.0, 0.0));

        // C far away (should not be neighbor)
        index.update_entity(uuid(30), cluster_c, Vec3::new(1000.0, 0.0, 0.0));

        // Build brute-force reference
        let mut clusters = HashMap::new();
        clusters.insert(
            cluster_a,
            vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)],
        );
        clusters.insert(
            cluster_b,
            vec![Vec3::new(80.0, 0.0, 0.0), Vec3::new(90.0, 0.0, 0.0)],
        );
        clusters.insert(cluster_c, vec![Vec3::new(1000.0, 0.0, 0.0)]);

        let brute_force_a = brute_force_neighbors(&clusters, 100.0, cluster_a);
        let grid_a = index.get_neighbors(cluster_a);

        assert_eq!(grid_a, brute_force_a, "A's neighbors must match");

        let brute_force_b = brute_force_neighbors(&clusters, 100.0, cluster_b);
        let grid_b = index.get_neighbors(cluster_b);

        assert_eq!(grid_b, brute_force_b, "B's neighbors must match");

        let brute_force_c = brute_force_neighbors(&clusters, 100.0, cluster_c);
        let grid_c = index.get_neighbors(cluster_c);

        assert_eq!(grid_c, brute_force_c, "C's neighbors must match");
    }

    #[test]
    fn grid_neighbors_matches_brute_force_varying_cell_sizes() {
        // Test with different cell sizes to ensure grid boundaries don't break correctness
        for cell_size in [10.0, 30.0, 75.0, 150.0] {
            let mut index = SpatialIndex::with_cell_size(cell_size);
            index.set_observation_radius(50.0);

            let cluster_a = uuid(1);
            let cluster_b = uuid(2);

            // Place clusters at positions that might fall on cell boundaries
            index.update_entity(uuid(10), cluster_a, Vec3::new(0.0, 0.0, 0.0));
            index.update_entity(uuid(20), cluster_b, Vec3::new(55.0, 0.0, 55.0));

            let mut clusters = HashMap::new();
            clusters.insert(cluster_a, vec![Vec3::new(0.0, 0.0, 0.0)]);
            clusters.insert(cluster_b, vec![Vec3::new(55.0, 0.0, 55.0)]);

            let brute_force = brute_force_neighbors(&clusters, 50.0, cluster_a);
            let grid = index.get_neighbors(cluster_a);

            assert_eq!(
                grid, brute_force,
                "neighbors must match for cell_size={}",
                cell_size
            );
        }
    }

    #[test]
    fn grid_placement_by_centroid_not_last_entity() {
        // Regression test: old code placed clusters by last-updated entity position,
        // not centroid. This could cause clusters to be placed far from their actual center,
        // leading to missing neighbors.
        let mut index = SpatialIndex::with_cell_size(50.0);
        index.set_observation_radius(10.0);

        let cluster_a = uuid(1);
        let cluster_b = uuid(2);

        // Cluster A: centered at (100, 0, 100) but last entity updated at far edge (150, 0, 150)
        index.update_entity(uuid(10), cluster_a, Vec3::new(100.0, 0.0, 100.0));
        index.update_entity(uuid(11), cluster_a, Vec3::new(110.0, 0.0, 110.0)); // Update at edge
        index.update_entity(uuid(12), cluster_a, Vec3::new(150.0, 0.0, 150.0)); // Last update at far edge

        // Cluster B: centered at (130, 0, 130), overlaps with A's centroid but not with A's last-updated position
        index.update_entity(uuid(20), cluster_b, Vec3::new(130.0, 0.0, 130.0));
        index.update_entity(uuid(21), cluster_b, Vec3::new(140.0, 0.0, 140.0));

        // If code placed cluster A by last-updated position (150, 0, 150), it would be in a different cell
        // and might miss cluster B as a neighbor. But with centroid-based placement, they should be neighbors.
        let mut clusters = HashMap::new();
        clusters.insert(
            cluster_a,
            vec![
                Vec3::new(100.0, 0.0, 100.0),
                Vec3::new(110.0, 0.0, 110.0),
                Vec3::new(150.0, 0.0, 150.0),
            ],
        );
        clusters.insert(
            cluster_b,
            vec![Vec3::new(130.0, 0.0, 130.0), Vec3::new(140.0, 0.0, 140.0)],
        );

        let brute_force_a = brute_force_neighbors(&clusters, 10.0, cluster_a);
        let grid_a = index.get_neighbors(cluster_a);

        // B should be in A's neighbors
        assert!(
            brute_force_a.contains(&cluster_b),
            "brute force must find B as neighbor of A"
        );
        assert_eq!(
            grid_a, brute_force_a,
            "grid must match brute force (B must be A's neighbor)"
        );
    }
}
