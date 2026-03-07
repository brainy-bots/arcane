//! Shared types used across Arcane components.

use uuid::Uuid;

/// Re-export for use in interface types.
pub use uuid::Uuid as EntityId;

/// 2D vector (e.g. centroid in 2D plane, or x/z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// 3D position (world position, centroid with height).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Squared distance to another point (avoids sqrt for comparisons).
    pub fn distance_sq_to(&self, other: &Vec3) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }
}

/// Per-cluster geometry from SpatialIndex (IN-03). Used for neighbor lists and WorldStateView.
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterGeometry {
    pub cluster_id: Uuid,
    pub centroid: Vec3,
    pub spread_radius: f64,
    pub entity_count: u32,
}
