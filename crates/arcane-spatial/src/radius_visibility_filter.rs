use arcane_core::{IVisibilityFilter, Vec3};
use uuid::Uuid;

/// L0 default visibility filter — radius-based area of interest (AOI).
///
/// Entities within the configured radius of the observer are visible;
/// those outside are filtered out. This is the baseline sensible default
/// every game gets without further configuration.
pub struct RadiusVisibilityFilter {
    radius_sq: f64,
}

impl RadiusVisibilityFilter {
    /// Create a new radius visibility filter with the given radius.
    ///
    /// # Arguments
    /// * `radius` — the visibility radius. Pre-squared internally for efficient distance checks.
    pub fn new(radius: f64) -> Self {
        Self {
            radius_sq: radius * radius,
        }
    }
}

impl IVisibilityFilter for RadiusVisibilityFilter {
    fn filter(&self, observer_position: Vec3, entities: &[(Uuid, Vec3)]) -> Vec<bool> {
        entities
            .iter()
            .map(|(_, pos)| {
                let distance_sq = observer_position.distance_sq_to(pos);
                distance_sq <= self.radius_sq
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_returns_correct_length() {
        let filter = RadiusVisibilityFilter::new(10.0);
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
        let filter = RadiusVisibilityFilter::new(10.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities = vec![(Uuid::nil(), Vec3::new(5.0, 0.0, 0.0))];

        let result = filter.filter(observer, &entities);
        assert!(result[0]);
    }

    #[test]
    fn filter_excludes_entities_outside_radius() {
        let filter = RadiusVisibilityFilter::new(10.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities = vec![(Uuid::nil(), Vec3::new(20.0, 0.0, 0.0))];

        let result = filter.filter(observer, &entities);
        assert!(!result[0]);
    }

    #[test]
    fn filter_handles_empty_entity_list() {
        let filter = RadiusVisibilityFilter::new(10.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities: Vec<(Uuid, Vec3)> = vec![];

        let result = filter.filter(observer, &entities);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn filter_on_boundary() {
        let filter = RadiusVisibilityFilter::new(10.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities = vec![(Uuid::nil(), Vec3::new(0.0, 10.0, 0.0))];

        let result = filter.filter(observer, &entities);
        assert!(result[0]); // On boundary should be visible
    }

    #[test]
    fn filter_zero_radius() {
        let filter = RadiusVisibilityFilter::new(0.0);
        let observer = Vec3::new(0.0, 0.0, 0.0);
        let entities = vec![
            (Uuid::nil(), Vec3::new(0.0, 0.0, 0.0)), // At observer position
            (Uuid::nil(), Vec3::new(0.001, 0.0, 0.0)), // Very close
        ];

        let result = filter.filter(observer, &entities);
        assert!(result[0]); // At exact position should be visible
        assert!(!result[1]); // Any distance > 0 should be outside radius
    }

    #[test]
    fn filter_observer_at_entity_position() {
        let filter = RadiusVisibilityFilter::new(5.0);
        let observer = Vec3::new(3.0, 4.0, 0.0);
        let entities = vec![(Uuid::nil(), Vec3::new(3.0, 4.0, 0.0))];

        let result = filter.filter(observer, &entities);
        assert!(result[0]); // Entity at observer position should be visible
    }

    #[test]
    fn filter_with_mixed_visibility() {
        let filter = RadiusVisibilityFilter::new(10.0);
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
