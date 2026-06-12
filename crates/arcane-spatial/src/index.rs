//! SpatialIndex — per-cluster geometry and neighbor discovery.
//!
//! API from IN-03: update_entity, remove_entity, set_observation_radius,
//! get_cluster_geometry, get_neighbors, get_clusters_in_region, snapshot_for_view.
//!
//! Internals (issue #169): a 3D sparse spatial hash over cluster centroids plus
//! incrementally-cached per-cluster geometry. Entity updates are O(1) amortized
//! (per-cluster running position sum, dirty-flag geometry cache, reverse
//! cluster→cell map so re-bucketing never sweeps the grid). Neighbor queries
//! touch only the grid cells within the query radius and use cached geometry.
//!
//! Distance is 3D with a configurable vertical weight (`y_weight`): 1.0 gives a
//! spherical metric, larger values shrink the effective vertical range (the
//! AOI-cylinder shape most games want), and 0.0 reproduces the legacy 2D
//! behavior. The weight applies consistently to the neighbor metric, the
//! internal spreads, and the cell mapping. `ClusterGeometry.spread_radius`
//! stays in unweighted world units (public API contract).

use arcane_core::types::{ClusterGeometry, Vec3};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Default grid cell edge length in world units (on the order of a typical observation radius).
const DEFAULT_CELL_SIZE: f64 = 50.0;

/// 3D sparse-hash cell key (weighted space).
#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
struct GridCell(i64, i64, i64);

/// Per-cluster entity bucket with incrementally-maintained aggregates.
struct ClusterBucket {
    /// entity_id -> world position.
    entities: HashMap<Uuid, Vec3>,
    /// Running sum of positions — centroid is O(1).
    position_sum: Vec3,
    /// Cached (world_spread, weighted_spread); None = dirty, recomputed lazily.
    /// Cell so read paths (&self) can refresh the cache — the index is a
    /// single-consumer structure per IN-03 (ArcaneManager owns it), not Sync.
    cached_spread: Cell<Option<(f64, f64)>>,
}

impl ClusterBucket {
    fn new() -> Self {
        Self {
            entities: HashMap::new(),
            position_sum: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            cached_spread: Cell::new(None),
        }
    }

    fn insert(&mut self, entity_id: Uuid, position: Vec3) {
        if let Some(old) = self.entities.insert(entity_id, position) {
            self.position_sum.x -= old.x;
            self.position_sum.y -= old.y;
            self.position_sum.z -= old.z;
        }
        self.position_sum.x += position.x;
        self.position_sum.y += position.y;
        self.position_sum.z += position.z;
        self.cached_spread.set(None);
    }

    fn remove(&mut self, entity_id: Uuid) -> bool {
        if let Some(old) = self.entities.remove(&entity_id) {
            self.position_sum.x -= old.x;
            self.position_sum.y -= old.y;
            self.position_sum.z -= old.z;
            self.cached_spread.set(None);
            true
        } else {
            false
        }
    }

    /// O(1) centroid from the running sum. Caller must ensure the bucket is non-empty.
    fn centroid(&self) -> Vec3 {
        let n = self.entities.len() as f64;
        Vec3 {
            x: self.position_sum.x / n,
            y: self.position_sum.y / n,
            z: self.position_sum.z / n,
        }
    }

    /// (world_spread, weighted_spread) from cache, recomputing only when dirty.
    fn spreads(&self, y_weight: f64) -> (f64, f64) {
        if let Some(cached) = self.cached_spread.get() {
            return cached;
        }
        let centroid = self.centroid();
        let mut world = 0.0_f64;
        let mut weighted = 0.0_f64;
        for p in self.entities.values() {
            let dx = p.x - centroid.x;
            let dy = p.y - centroid.y;
            let dz = p.z - centroid.z;
            world = world.max((dx * dx + dy * dy + dz * dz).sqrt());
            let wy = dy * y_weight;
            weighted = weighted.max((dx * dx + wy * wy + dz * dz).sqrt());
        }
        self.cached_spread.set(Some((world, weighted)));
        (world, weighted)
    }
}

/// 3D coarse spatial index over cluster entities. Caller (e.g. ArcaneManager) feeds
/// entity positions via update_entity / remove_entity; index answers geometry and neighbor queries.
pub struct SpatialIndex {
    observation_radius: f64,
    /// Grid cell edge length in (weighted) world units. Config field.
    grid_cell_size: f64,
    /// Vertical distance weight. 1.0 = sphere, >1.0 = tighter vertical range, 0.0 = legacy 2D. Config field.
    y_weight: f64,
    /// cluster_id -> entity bucket with cached aggregates.
    clusters: HashMap<Uuid, ClusterBucket>,
    /// entity_id -> cluster_id reverse map (O(1) cluster lookup on update/remove).
    entity_to_cluster: HashMap<Uuid, Uuid>,
    /// Sparse hash: centroid cell -> cluster_ids whose centroid falls in that cell.
    grid: HashMap<GridCell, HashSet<Uuid>>,
    /// cluster_id -> its current centroid cell (O(1) re-bucketing, no grid sweeps).
    cluster_to_cell: HashMap<Uuid, GridCell>,
}

impl SpatialIndex {
    /// Create a new index with default cell size. Call set_observation_radius before get_neighbors.
    pub fn new() -> Self {
        Self::with_config(DEFAULT_CELL_SIZE, 1.0)
    }

    /// Create a new index with explicit cell size and vertical weight (config fields).
    pub fn with_config(cell_size: f64, y_weight: f64) -> Self {
        Self {
            observation_radius: 0.0,
            grid_cell_size: cell_size.max(1.0),
            y_weight: y_weight.max(0.0),
            clusters: HashMap::new(),
            entity_to_cluster: HashMap::new(),
            grid: HashMap::new(),
            cluster_to_cell: HashMap::new(),
        }
    }

    /// Set observation radius used for get_neighbors() effective area. Typically from config.
    pub fn set_observation_radius(&mut self, radius: f64) {
        self.observation_radius = radius;
    }

    fn cell_for(&self, p: Vec3) -> GridCell {
        GridCell(
            (p.x / self.grid_cell_size).floor() as i64,
            (p.y * self.y_weight / self.grid_cell_size).floor() as i64,
            (p.z / self.grid_cell_size).floor() as i64,
        )
    }

    /// Weighted distance between two points: full 3D with `y_weight` on the vertical axis.
    fn weighted_distance(&self, a: Vec3, b: Vec3) -> f64 {
        let dx = a.x - b.x;
        let dy = (a.y - b.y) * self.y_weight;
        let dz = a.z - b.z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    /// Move a cluster's grid registration to the cell of its current centroid. O(1).
    fn rebucket(&mut self, cluster_id: Uuid) {
        let Some(bucket) = self.clusters.get(&cluster_id) else {
            // Cluster emptied: drop its grid registration.
            if let Some(old_cell) = self.cluster_to_cell.remove(&cluster_id) {
                if let Some(cell) = self.grid.get_mut(&old_cell) {
                    cell.remove(&cluster_id);
                    if cell.is_empty() {
                        self.grid.remove(&old_cell);
                    }
                }
            }
            return;
        };
        let new_cell = self.cell_for(bucket.centroid());
        match self.cluster_to_cell.get(&cluster_id) {
            Some(&old_cell) if old_cell == new_cell => {}
            Some(&old_cell) => {
                if let Some(cell) = self.grid.get_mut(&old_cell) {
                    cell.remove(&cluster_id);
                    if cell.is_empty() {
                        self.grid.remove(&old_cell);
                    }
                }
                self.grid.entry(new_cell).or_default().insert(cluster_id);
                self.cluster_to_cell.insert(cluster_id, new_cell);
            }
            None => {
                self.grid.entry(new_cell).or_default().insert(cluster_id);
                self.cluster_to_cell.insert(cluster_id, new_cell);
            }
        }
    }

    /// Register or update an entity's position and cluster. If cluster_id changed, updates both clusters.
    pub fn update_entity(&mut self, entity_id: Uuid, cluster_id: Uuid, position: Vec3) {
        if let Some(&old_cluster) = self.entity_to_cluster.get(&entity_id) {
            if old_cluster != cluster_id {
                let emptied = match self.clusters.get_mut(&old_cluster) {
                    Some(bucket) => {
                        bucket.remove(entity_id);
                        bucket.entities.is_empty()
                    }
                    None => false,
                };
                if emptied {
                    self.clusters.remove(&old_cluster);
                }
                self.rebucket(old_cluster);
            }
        }
        self.clusters
            .entry(cluster_id)
            .or_insert_with(ClusterBucket::new)
            .insert(entity_id, position);
        self.entity_to_cluster.insert(entity_id, cluster_id);
        self.rebucket(cluster_id);
    }

    /// Remove an entity (despawn or reassignment). Updates that cluster's centroid and spread.
    pub fn remove_entity(&mut self, entity_id: Uuid, _cluster_id: Uuid) {
        let Some(cluster_id) = self.entity_to_cluster.remove(&entity_id) else {
            return;
        };
        let emptied = match self.clusters.get_mut(&cluster_id) {
            Some(bucket) => {
                bucket.remove(entity_id);
                bucket.entities.is_empty()
            }
            None => false,
        };
        if emptied {
            self.clusters.remove(&cluster_id);
        }
        self.rebucket(cluster_id);
    }

    /// Return centroid, spread_radius, and entity_count for a cluster, or None if not in index.
    /// Centroid and spread are in unweighted world units (public API contract).
    pub fn get_cluster_geometry(&self, cluster_id: Uuid) -> Option<ClusterGeometry> {
        let bucket = self.clusters.get(&cluster_id)?;
        if bucket.entities.is_empty() {
            return None;
        }
        let centroid = bucket.centroid();
        let (world_spread, _) = bucket.spreads(self.y_weight);
        Some(ClusterGeometry {
            cluster_id,
            centroid,
            spread_radius: world_spread,
            entity_count: bucket.entities.len() as u32,
        })
    }

    /// Return cluster_ids whose effective area (centroid + spread_radius + observation_radius)
    /// overlaps this cluster's, under the weighted 3D metric.
    pub fn get_neighbors(&self, cluster_id: Uuid) -> Vec<Uuid> {
        let y_weight = self.y_weight;
        let (self_centroid, self_weighted_spread) = match self.clusters.get(&cluster_id) {
            Some(bucket) if !bucket.entities.is_empty() => {
                let (_, weighted) = bucket.spreads(y_weight);
                (bucket.centroid(), weighted)
            }
            _ => return vec![],
        };
        let effective_self = self_weighted_spread + self.observation_radius;

        // Candidate search radius must cover: our effective area + the other cluster's effective
        // area + the other centroid's possible offset within its cell. Refresh the max weighted
        // spread from caches (recomputes only dirty buckets — amortized O(1) per mutation).
        let mut max_other_spread = 0.0_f64;
        for (id, bucket) in self.clusters.iter() {
            if *id == cluster_id || bucket.entities.is_empty() {
                continue;
            }
            let (_, weighted) = bucket.spreads(y_weight);
            max_other_spread = max_other_spread.max(weighted);
        }
        let search_radius = effective_self + max_other_spread + self.observation_radius;

        // Cells within the search radius around our centroid's cell (weighted space).
        let center_cell = self.cell_for(self_centroid);
        let reach = (search_radius / self.grid_cell_size).ceil() as i64 + 1;
        let cube_cells = (2 * reach + 1).pow(3) as usize;

        let mut candidates: HashSet<Uuid> = HashSet::new();
        if cube_cells > self.grid.len() {
            // Search cube exceeds the number of occupied cells (huge spreads or tiny cells):
            // walking the sparse grid directly is cheaper than enumerating the cube.
            for (cell, ids) in &self.grid {
                let dx = (cell.0 - center_cell.0).abs();
                let dy = (cell.1 - center_cell.1).abs();
                let dz = (cell.2 - center_cell.2).abs();
                if dx <= reach && dy <= reach && dz <= reach {
                    candidates.extend(ids.iter().copied());
                }
            }
        } else {
            for dx in -reach..=reach {
                for dy in -reach..=reach {
                    for dz in -reach..=reach {
                        let cell =
                            GridCell(center_cell.0 + dx, center_cell.1 + dy, center_cell.2 + dz);
                        if let Some(ids) = self.grid.get(&cell) {
                            candidates.extend(ids.iter().copied());
                        }
                    }
                }
            }
        }
        candidates.remove(&cluster_id);

        let mut neighbors: Vec<Uuid> = Vec::new();
        for other_id in candidates {
            let Some(bucket) = self.clusters.get(&other_id) else {
                continue;
            };
            if bucket.entities.is_empty() {
                continue;
            }
            let (_, other_weighted_spread) = bucket.spreads(y_weight);
            let other_centroid = bucket.centroid();
            let effective_other = other_weighted_spread + self.observation_radius;
            if self.weighted_distance(self_centroid, other_centroid)
                <= effective_self + effective_other
            {
                neighbors.push(other_id);
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
                bucket
                    .entities
                    .values()
                    .any(|p| {
                        let dx = p.x - cx;
                        let dz = p.z - cz;
                        dx * dx + dz * dz <= r_sq
                    })
                    .then_some(*cluster_id)
            })
            .collect();
        cluster_ids.sort();
        cluster_ids
    }

    /// Return all entities as (entity_id, cluster_id, position) triples.
    /// Used by ArcaneManager to populate WorldStateView.players.
    pub fn snapshot_entities(&self) -> Vec<(Uuid, Uuid, Vec3)> {
        let mut result: Vec<(Uuid, Uuid, Vec3)> = self
            .clusters
            .iter()
            .flat_map(|(&cluster_id, bucket)| {
                bucket
                    .entities
                    .iter()
                    .map(move |(&entity_id, &position)| (entity_id, cluster_id, position))
            })
            .collect();
        result.sort_by_key(|&(entity_id, _, _)| entity_id);
        result
    }

    /// Snapshot of all clusters for building WorldStateView. Called by ArcaneManager before evaluate().
    pub fn snapshot_for_view(&self) -> Vec<ClusterGeometry> {
        let mut cluster_ids: Vec<Uuid> = self.clusters.keys().copied().collect();
        cluster_ids.sort();
        cluster_ids
            .into_iter()
            .filter_map(|id| self.get_cluster_geometry(id))
            .collect()
    }
}

impl Default for SpatialIndex {
    fn default() -> Self {
        Self::new()
    }
}
