//! IVisibilityFilter (IF-02) — per-client visibility filtering in the outbound pipeline.
//!
//! Consumed by `arcane-infra::assemble_outbound_frame()` to select which entity chunks
//! a client should receive based on the observer's position. This is the first stage in
//! the outbound pipeline and sets the template pattern for all platform primitives.

use crate::types::Vec3;
use uuid::Uuid;

/// Per-client visibility filter. Called once per subscriber per tick
/// during outbound frame assembly to select which entity chunks
/// this client should receive.
pub trait IVisibilityFilter: Send + Sync {
    /// Filter entities based on observer position.
    ///
    /// Takes an observer position and a list of entities (as uuid/position pairs) and returns
    /// a boolean vector indicating which entities should be visible to the observer.
    ///
    /// # Arguments
    /// * `observer_position` — the position of the observer (client)
    /// * `entities` — a slice of (entity_id, position) tuples for all entities in scope
    ///
    /// # Returns
    /// A vector of booleans with the same length as `entities`, where `true` means
    /// the corresponding entity should be sent to the client, and `false` means it
    /// should be filtered out.
    fn filter(&self, observer_position: Vec3, entities: &[(Uuid, Vec3)]) -> Vec<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock visibility filter that accepts all entities within a fixed radius.
    struct MockRadiusFilter {
        radius: f64,
    }

    impl MockRadiusFilter {
        fn new(radius: f64) -> Self {
            Self { radius }
        }
    }

    impl IVisibilityFilter for MockRadiusFilter {
        fn filter(&self, observer_position: Vec3, entities: &[(Uuid, Vec3)]) -> Vec<bool> {
            entities
                .iter()
                .map(|(_, pos)| {
                    let distance_sq = observer_position.distance_sq_to(pos);
                    distance_sq <= self.radius * self.radius
                })
                .collect()
        }
    }

    #[test]
    fn filter_returns_correct_length() {
        let filter = MockRadiusFilter::new(10.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities = vec![
            (Uuid::nil(), Vec3::new(1.0, 0.0, 0.0)),
            (Uuid::nil(), Vec3::new(0.0, 1.0, 0.0)),
            (Uuid::nil(), Vec3::new(20.0, 0.0, 0.0)),
        ];

        let result = filter.filter(observer, &entities);
        assert_eq!(result.len(), entities.len());
    }

    #[test]
    fn filter_includes_entities_within_radius() {
        let filter = MockRadiusFilter::new(10.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities = vec![
            (Uuid::nil(), Vec3::new(5.0, 0.0, 0.0)), // distance = 5.0, within radius
        ];

        let result = filter.filter(observer, &entities);
        assert!(result[0]);
    }

    #[test]
    fn filter_excludes_entities_outside_radius() {
        let filter = MockRadiusFilter::new(10.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities = vec![
            (Uuid::nil(), Vec3::new(20.0, 0.0, 0.0)), // distance = 20.0, outside radius
        ];

        let result = filter.filter(observer, &entities);
        assert!(!result[0]);
    }

    #[test]
    fn filter_handles_empty_entity_list() {
        let filter = MockRadiusFilter::new(10.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities: Vec<(Uuid, Vec3)> = vec![];

        let result = filter.filter(observer, &entities);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn filter_with_mixed_visibility() {
        let filter = MockRadiusFilter::new(10.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities = vec![
            (Uuid::nil(), Vec3::new(3.0, 4.0, 0.0)), // distance = 5.0, within
            (Uuid::nil(), Vec3::new(15.0, 0.0, 0.0)), // distance = 15.0, outside
            (Uuid::nil(), Vec3::new(0.0, 10.0, 0.0)), // distance = 10.0, on boundary
        ];

        let result = filter.filter(observer, &entities);
        assert_eq!(result, vec![true, false, true]);
    }
}
