use crate::predictor::{GameFeatureProvider, InteractionPredictor, LinkKind, PairFeatures};
use arcane_core::types::Vec2;
use uuid::Uuid;

/// Candidate linkable cold pair awaiting promotion.
#[derive(Clone, Copy, Debug)]
pub struct ColdCandidate {
    /// First entity UUID.
    pub a: Uuid,
    /// Second entity UUID.
    pub b: Uuid,
    /// Position of entity a.
    pub pos_a: Vec2,
    /// Position of entity b.
    pub pos_b: Vec2,
    /// Velocity of entity a.
    pub vel_a: Vec2,
    /// Velocity of entity b.
    pub vel_b: Vec2,
    /// Latent link kind that qualified this candidate.
    pub link: LinkKind,
    /// Existing interaction-graph weight (0.0 if cold).
    pub history_weight: f64,
}

/// Configuration for the cold-pair sweep.
#[derive(Clone, Copy, Debug)]
pub struct SweepConfig {
    /// Prediction horizon T in seconds (default 5.0).
    pub horizon_secs: f64,
    /// Minimum predicted p to promote (default 0.1).
    pub promote_threshold: f64,
}

impl SweepConfig {
    pub fn new(horizon_secs: f64, promote_threshold: f64) -> Self {
        Self {
            horizon_secs,
            promote_threshold,
        }
    }
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            horizon_secs: 5.0,
            promote_threshold: 0.1,
        }
    }
}

/// Result of promotion: a pair and its predicted probability.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Promotion {
    /// First entity UUID.
    pub a: Uuid,
    /// Second entity UUID.
    pub b: Uuid,
    /// Predicted probability of interaction.
    pub p: f64,
}

/// Computes closing speed for a pair given their positions and velocities.
///
/// Returns the component of relative velocity along the line from a to b
/// that reduces distance. Positive means they are approaching each other.
pub fn closing_speed(pos_a: Vec2, pos_b: Vec2, vel_a: Vec2, vel_b: Vec2) -> f64 {
    let rel_pos = Vec2::new(pos_b.x - pos_a.x, pos_b.y - pos_a.y);
    let rel_vel = Vec2::new(vel_b.x - vel_a.x, vel_b.y - vel_a.y);

    let distance_sq = rel_pos.x * rel_pos.x + rel_pos.y * rel_pos.y;

    if distance_sq <= 1e-18 {
        // If distance is effectively zero, no meaningful closing speed
        0.0
    } else {
        let distance = distance_sq.sqrt();
        let dot_product = rel_vel.x * rel_pos.x + rel_vel.y * rel_pos.y;
        -dot_product / distance
    }
}

/// Sweeps cold-pair candidates and returns promotions.
///
/// For each candidate, computes the predicted interaction probability using the predictor.
/// Only pairs whose predicted p >= threshold are promoted (returned).
/// This is a pure function with zero-cost-on-zero: pairs below threshold contribute no output.
pub fn sweep_cold_pairs<P, G>(
    candidates: &[ColdCandidate],
    predictor: &P,
    features: &G,
    config: &SweepConfig,
) -> Vec<Promotion>
where
    P: InteractionPredictor,
    G: GameFeatureProvider,
{
    let mut promotions = Vec::new();

    for candidate in candidates {
        // 1. Compute distance
        let dx = candidate.pos_b.x - candidate.pos_a.x;
        let dy = candidate.pos_b.y - candidate.pos_a.y;
        let distance = (dx * dx + dy * dy).sqrt();

        // 2. Compute closing speed
        let cs = closing_speed(
            candidate.pos_a,
            candidate.pos_b,
            candidate.vel_a,
            candidate.vel_b,
        );

        // 3. Build PairFeatures
        let pf = PairFeatures {
            distance,
            closing_speed: cs,
            horizon_secs: config.horizon_secs,
            history_weight: candidate.history_weight,
            latent_link: Some(candidate.link),
            game: features.features_for_pair(candidate.a, candidate.b),
        };

        // 4. Predict
        let p = predictor.predict(&pf);

        // 5. Promote if p >= threshold (zero-cost-on-zero)
        if p >= config.promote_threshold {
            promotions.push(Promotion {
                a: candidate.a,
                b: candidate.b,
                p,
            });
        }
    }

    promotions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predictor::{HeuristicPredictor, NullFeatureProvider};
    use uuid::Uuid;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn test_empty_input() {
        let predictor = HeuristicPredictor::default();
        let features = NullFeatureProvider;
        let config = SweepConfig::default();

        let result = sweep_cold_pairs(&[], &predictor, &features, &config);

        assert!(result.is_empty());
    }

    #[test]
    fn test_teleport_promotion() {
        // A far-apart pair with a latent link (Party) should be promoted
        // because the link prior gives it p >= threshold even without geometric convergence.
        let predictor = HeuristicPredictor::default();
        let features = NullFeatureProvider;
        let config = SweepConfig::default();

        let candidate = ColdCandidate {
            a: uuid(1),
            b: uuid(2),
            pos_a: Vec2::new(0.0, 0.0),
            pos_b: Vec2::new(100000.0, 0.0),
            vel_a: Vec2::new(0.0, 0.0),
            vel_b: Vec2::new(0.0, 0.0),
            link: LinkKind::Party,
            history_weight: 0.0,
        };

        let result = sweep_cold_pairs(&[candidate], &predictor, &features, &config);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].a, uuid(1));
        assert_eq!(result[0].b, uuid(2));
        assert!(result[0].p >= config.promote_threshold);
    }

    #[test]
    fn test_cold_stays_cold() {
        // An unlinked far pair should NOT be promoted because geometry alone
        // doesn't achieve the threshold, and there's no latent link.
        let predictor = HeuristicPredictor::default();
        let features = NullFeatureProvider;
        let config = SweepConfig::new(5.0, 0.99);

        let candidate = ColdCandidate {
            a: uuid(1),
            b: uuid(2),
            pos_a: Vec2::new(0.0, 0.0),
            pos_b: Vec2::new(100000.0, 0.0),
            vel_a: Vec2::new(0.0, 0.0),
            vel_b: Vec2::new(0.0, 0.0),
            link: LinkKind::Party,
            history_weight: 0.0,
        };

        let result = sweep_cold_pairs(&[candidate], &predictor, &features, &config);

        // Even with a link, the high threshold (0.99) means it stays cold.
        assert!(result.is_empty());
    }

    #[test]
    fn test_closing_speed_effect() {
        // Two identical pairs: one approaching, one separating.
        // The approaching pair should have higher predicted p.
        let predictor = HeuristicPredictor::default();
        let features = NullFeatureProvider;
        let config = SweepConfig::default();

        let a = uuid(1);
        let b = uuid(2);

        // Pair 1: approaching (closing_speed > 0)
        let candidate_approaching = ColdCandidate {
            a,
            b,
            pos_a: Vec2::new(0.0, 0.0),
            pos_b: Vec2::new(100.0, 0.0),
            vel_a: Vec2::new(5.0, 0.0),
            vel_b: Vec2::new(0.0, 0.0),
            link: LinkKind::Party,
            history_weight: 0.0,
        };

        // Pair 2: separating (closing_speed < 0)
        let candidate_separating = ColdCandidate {
            a,
            b,
            pos_a: Vec2::new(0.0, 0.0),
            pos_b: Vec2::new(100.0, 0.0),
            vel_a: Vec2::new(-5.0, 0.0),
            vel_b: Vec2::new(0.0, 0.0),
            link: LinkKind::Party,
            history_weight: 0.0,
        };

        let result_approaching =
            sweep_cold_pairs(&[candidate_approaching], &predictor, &features, &config);
        let result_separating =
            sweep_cold_pairs(&[candidate_separating], &predictor, &features, &config);

        assert_eq!(result_approaching.len(), 1);
        assert_eq!(result_separating.len(), 1);

        let p_approaching = result_approaching[0].p;
        let p_separating = result_separating[0].p;

        assert!(
            p_approaching >= p_separating,
            "approaching pair should have p >= separating pair; p_approaching={}, p_separating={}",
            p_approaching,
            p_separating
        );
    }

    #[test]
    fn test_zero_cost_on_zero() {
        // A candidate whose predicted p is below threshold should contribute no output.
        let predictor = HeuristicPredictor::default();
        let features = NullFeatureProvider;
        let config = SweepConfig::new(5.0, 0.99);

        let candidate = ColdCandidate {
            a: uuid(1),
            b: uuid(2),
            pos_a: Vec2::new(0.0, 0.0),
            pos_b: Vec2::new(100.0, 0.0),
            vel_a: Vec2::new(0.0, 0.0),
            vel_b: Vec2::new(0.0, 0.0),
            link: LinkKind::Party,
            history_weight: 0.0,
        };

        let result = sweep_cold_pairs(&[candidate], &predictor, &features, &config);

        assert!(
            result.is_empty(),
            "below-threshold candidate should produce no output"
        );
    }

    #[test]
    fn test_closing_speed_math() {
        // Test the closing_speed computation directly.
        // Case: a at (0,0), b at (10,0), a moving toward b at speed 1.
        let pos_a = Vec2::new(0.0, 0.0);
        let pos_b = Vec2::new(10.0, 0.0);
        let vel_a = Vec2::new(1.0, 0.0);
        let vel_b = Vec2::new(0.0, 0.0);

        let cs = closing_speed(pos_a, pos_b, vel_a, vel_b);

        // a is chasing b. rel_pos = (10, 0), rel_vel = (-1, 0).
        // dot(rel_vel, rel_pos) = -1 * 10 + 0 * 0 = -10.
        // distance = 10.
        // closing_speed = -(-10) / 10 = 1.0
        assert!(
            (cs - 1.0).abs() < 1e-9,
            "closing_speed should be ~1.0, got {}",
            cs
        );
    }

    #[test]
    fn test_zero_distance_edge_case() {
        // When distance is effectively zero, closing speed should be 0.
        let pos_a = Vec2::new(0.0, 0.0);
        let pos_b = Vec2::new(1e-10, 1e-10);
        let vel_a = Vec2::new(1.0, 0.0);
        let vel_b = Vec2::new(0.0, 0.0);

        let cs = closing_speed(pos_a, pos_b, vel_a, vel_b);

        assert!(
            (cs - 0.0).abs() < 1e-9,
            "closing_speed for nearly-zero distance should be 0, got {}",
            cs
        );
    }

    #[test]
    fn test_multiple_candidates_order_preserved() {
        // Multiple candidates should be returned in order.
        let predictor = HeuristicPredictor::default();
        let features = NullFeatureProvider;
        let config = SweepConfig::default();

        let mut promotions = Vec::new();

        for i in 0u8..5 {
            let candidate = ColdCandidate {
                a: uuid(i),
                b: uuid(i + 100),
                pos_a: Vec2::new(0.0, 0.0),
                pos_b: Vec2::new(100000.0, 0.0),
                vel_a: Vec2::new(0.0, 0.0),
                vel_b: Vec2::new(0.0, 0.0),
                link: LinkKind::Party,
                history_weight: 0.0,
            };
            promotions.push(candidate);
        }

        let result = sweep_cold_pairs(&promotions, &predictor, &features, &config);

        // All should be promoted (link_prior >= threshold)
        assert_eq!(result.len(), 5);

        // Check order is preserved
        for (i, promotion) in result.iter().enumerate() {
            assert_eq!(promotion.a, uuid(i as u8));
            assert_eq!(promotion.b, uuid(i as u8 + 100));
        }
    }

    #[test]
    fn test_promotion_values_are_meaningful() {
        // Verify that promotion.p values are within [0.0, 1.0] and sensible.
        let predictor = HeuristicPredictor::default();
        let features = NullFeatureProvider;
        let config = SweepConfig::default();

        let candidate = ColdCandidate {
            a: uuid(1),
            b: uuid(2),
            pos_a: Vec2::new(0.0, 0.0),
            pos_b: Vec2::new(100000.0, 0.0),
            vel_a: Vec2::new(0.0, 0.0),
            vel_b: Vec2::new(0.0, 0.0),
            link: LinkKind::Party,
            history_weight: 0.0,
        };

        let result = sweep_cold_pairs(&[candidate], &predictor, &features, &config);

        assert_eq!(result.len(), 1);
        let p = result[0].p;
        assert!((0.0..=1.0).contains(&p), "p must be in [0, 1], got {}", p);
    }
}
