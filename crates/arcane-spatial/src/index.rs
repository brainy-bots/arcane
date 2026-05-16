//! SpatialIndex — per-cluster geometry and neighbor discovery.
//!
//! API from IN-03: update_entity, remove_entity, set_observation_radius,
//! get_cluster_geometry, get_neighbors, get_clusters_in_region, snapshot_for_view.

use arcane_core::types::{ClusterGeometry, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

/// 2D coarse spatial index over cluster entities. Caller (e.g. ArcaneManager) feeds
/// entity positions via update_entity / remove_entity; index answers geometry and neighbor queries.
pub struct SpatialIndex {
    observation_radius: f64,
    /// entity_id -> (cluster_id, position). One cluster per entity; moving entity = update with new cluster_id.
    entities: HashMap<Uuid, (Uuid, Vec3)>,
}

impl SpatialIndex {
    /// Create a new index. Call set_observation_radius before get_neighbors.
    pub fn new() -> Self {
        Self {
            observation_radius: 0.0,
            entities: HashMap::new(),
        }
    }

    /// Register or update an entity's position and cluster. If cluster_id changed, updates both clusters.
    pub fn update_entity(&mut self, entity_id: Uuid, cluster_id: Uuid, position: Vec3) {
        self.entities.insert(entity_id, (cluster_id, position));
    }

    /// Remove an entity (despawn or reassignment). Updates that cluster's centroid and spread.
    pub fn remove_entity(&mut self, entity_id: Uuid, _cluster_id: Uuid) {
        self.entities.remove(&entity_id);
    }

    /// Set observation radius used for get_neighbors() effective area. Typically from config.
    pub fn set_observation_radius(&mut self, radius: f64) {
        self.observation_radius = radius;
    }

    /// Return centroid, spread_radius, and entity_count for a cluster, or None if not in index.
    pub fn get_cluster_geometry(&self, cluster_id: Uuid) -> Option<ClusterGeometry> {
        let positions: Vec<Vec3> = self
            .entities
            .values()
            .filter(|(c, _)| *c == cluster_id)
            .map(|(_, p)| *p)
            .collect();
        if positions.is_empty() {
            return None;
        }
        let n = positions.len() as f64;
        let centroid = Vec3 {
            x: positions.iter().map(|p| p.x).sum::<f64>() / n,
            y: positions.iter().map(|p| p.y).sum::<f64>() / n,
            z: positions.iter().map(|p| p.z).sum::<f64>() / n,
        };
        let spread_radius = positions
            .iter()
            .map(|p| p.distance_sq_to(&centroid).sqrt())
            .fold(0.0_f64, f64::max);
        Some(ClusterGeometry {
            cluster_id,
            centroid,
            spread_radius,
            entity_count: positions.len() as u32,
        })
    }

    /// Return cluster_ids whose effective area (centroid + spread_radius + observation_radius) overlaps this cluster's.
    pub fn get_neighbors(&self, cluster_id: Uuid) -> Vec<Uuid> {
        let geom = match self.get_cluster_geometry(cluster_id) {
            Some(g) => g,
            None => return vec![],
        };
        let effective_self = geom.spread_radius + self.observation_radius;
        let snapshot = self.snapshot_for_view();
        let mut neighbors = Vec::new();
        for other in snapshot {
            if other.cluster_id == cluster_id {
                continue;
            }
            let effective_other = other.spread_radius + self.observation_radius;
            let dx = geom.centroid.x - other.centroid.x;
            let dz = geom.centroid.z - other.centroid.z;
            let dist_2d = (dx * dx + dz * dz).sqrt();
            if dist_2d <= effective_self + effective_other {
                neighbors.push(other.cluster_id);
            }
        }
        neighbors
    }

    /// Return cluster_ids that have any entity in the given 2D region (center x/z, radius). Optional API.
    pub fn get_clusters_in_region(&self, center: (f64, f64), radius: f64) -> Vec<Uuid> {
        let (cx, cz) = center;
        let r_sq = radius * radius;
        let mut cluster_ids: Vec<Uuid> = self
            .entities
            .values()
            .filter(|(_, p)| {
                let dx = p.x - cx;
                let dz = p.z - cz;
                dx * dx + dz * dz <= r_sq
            })
            .map(|(c, _)| *c)
            .collect();
        cluster_ids.sort();
        cluster_ids.dedup();
        cluster_ids
    }

    /// Return all entities as (entity_id, cluster_id, position) triples.
    /// Used by ArcaneManager to populate WorldStateView.players.
    pub fn snapshot_entities(&self) -> Vec<(Uuid, Uuid, Vec3)> {
        self.entities
            .iter()
            .map(|(&entity_id, &(cluster_id, position))| (entity_id, cluster_id, position))
            .collect()
    }

    /// Snapshot of all clusters for building WorldStateView. Called by ArcaneManager before evaluate().
    pub fn snapshot_for_view(&self) -> Vec<ClusterGeometry> {
        let mut cluster_ids: Vec<Uuid> = self.entities.values().map(|(c, _)| *c).collect();
        cluster_ids.sort();
        cluster_ids.dedup();
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
