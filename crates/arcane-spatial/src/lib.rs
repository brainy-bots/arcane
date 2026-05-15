//! Arcane Engine — SpatialIndex (IN-03).
//!
//! Maintains a 2D coarse spatial index over cluster entities. Exposes centroid,
//! spread radius, and neighbor queries. No I/O; caller feeds data via update_entity / remove_entity.

mod index;
mod radius_visibility_filter;

pub use arcane_core::ClusterGeometry;
pub use index::SpatialIndex;
pub use radius_visibility_filter::RadiusVisibilityFilter;
