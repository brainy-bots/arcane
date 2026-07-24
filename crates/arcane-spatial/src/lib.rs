//! Arcane Engine — SpatialIndex (IN-03).
//!
//! Maintains a 3D sparse spatial hash over cluster entities. Exposes centroid,
//! spread radius, and neighbor queries. No I/O; caller feeds data via update_entity / remove_entity.

mod index;

pub use arcane_core::ClusterGeometry;
pub use index::SpatialIndex;
