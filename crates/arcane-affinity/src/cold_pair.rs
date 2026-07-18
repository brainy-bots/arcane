use crate::predictor::{GameFeatureProvider, InteractionPredictor, LinkKind, PairContext};
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

        // 3. Fetch game features for this pair
        let features_a = features.features_for_pair(candidate.a, candidate.b);
        let features_b = features.features_for_pair(candidate.b, candidate.a);

        // 4. Build PairContext (note: LinkKind is not used in new API)
        // TODO(#272-A4): removed with the sweep rewrite
        let ctx = PairContext {
            distance,
            closing_speed: cs,
            horizon_secs: config.horizon_secs,
            history_weight: candidate.history_weight,
            features_a: &features_a,
            features_b: &features_b,
        };

        // 5. Predict
        let p = predictor.predict(&ctx);

        // 6. Promote if p >= threshold (zero-cost-on-zero)
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
    fn test_closing_speed_effect() {
        // Directly test that closing speed affects prediction probability.
        let predictor = HeuristicPredictor::default();
        let features_a = crate::feature_map::FeatureMap::new();
        let features_b = crate::feature_map::FeatureMap::new();

        let ctx_stationary = PairContext {
            distance: 50.0,
            closing_speed: 0.0,
            horizon_secs: 5.0,
            history_weight: 0.0,
            features_a: &features_a,
            features_b: &features_b,
        };

        let ctx_closing = PairContext {
            distance: 50.0,
            closing_speed: 5.0,
            horizon_secs: 5.0,
            history_weight: 0.0,
            features_a: &features_a,
            features_b: &features_b,
        };

        let p_stationary = predictor.predict(&ctx_stationary);
        let p_closing = predictor.predict(&ctx_closing);

        assert!(
            p_closing > p_stationary,
            "closing pair should have higher p than stationary; p_closing={}, p_stationary={}",
            p_closing,
            p_stationary
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
}
