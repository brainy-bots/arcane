//! Shared types used across Arcane components.
//!
//! These are intentionally minimal and serialization-friendly so both rule/infra code and tests
//! can share a single geometry vocabulary.

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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn vec3_distance_sq_symmetric_and_zero_for_same_point() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, 5.0, 6.0);
        assert_eq!(a.distance_sq_to(&b), b.distance_sq_to(&a));
        assert_eq!(a.distance_sq_to(&a), 0.0);
        let expected = 9.0 + 9.0 + 9.0; // (4-1)^2 + (5-2)^2 + (6-3)^2
        assert!((a.distance_sq_to(&b) - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn vec3_serde_roundtrip() {
        let v = Vec3::new(-1.5, 0.0, 2.25);
        let json = serde_json::to_string(&v).unwrap();
        let w: Vec3 = serde_json::from_str(&json).unwrap();
        assert_eq!(v, w);
    }

    #[test]
    fn cluster_geometry_partial_eq() {
        let id = Uuid::nil();
        let g1 = ClusterGeometry {
            cluster_id: id,
            centroid: Vec3::new(0., 1., 2.),
            spread_radius: 10.5,
            entity_count: 42,
        };
        let g2 = ClusterGeometry {
            cluster_id: id,
            centroid: Vec3::new(0., 1., 2.),
            spread_radius: 10.5,
            entity_count: 42,
        };
        assert_eq!(g1, g2);
    }
}
