use crate::feature_map::FeatureMap;
use crate::interaction_graph::InteractionGraph;
use crate::predictor::{InteractionPredictor, PairContext};
use arcane_core::types::Vec2;
use uuid::Uuid;

/// Candidate pair detected during the SCREEN pass: cheap spatial + graph checks.
#[derive(Clone, Copy, Debug)]
pub struct ScreenCandidate {
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

/// Screen pass: cheap spatial + graph heuristics to find candidate pairs.
///
/// Sources:
/// (a) Spatial convergence: pairs within screen_radius whose closing_speed >= min_closing_speed, not already graph-connected.
/// (b) Graph adjacency: neighbors-of-neighbors (a-b and b-c edges exist, a-c does not).
/// (c) Shared declared features: for each edge rule, pairs sharing an equal feature value, not already graph-connected.
pub fn screen_candidates(
    players: &[(Uuid, Vec2, Vec2)],
    features: &[(Uuid, FeatureMap)],
    graph: &InteractionGraph,
    screen_radius: f64,
    min_closing_speed: f64,
    edge_rules: &[(String, f64)],
) -> Vec<ScreenCandidate> {
    let mut candidates = Vec::new();
    let hot_floor = 1.0;

    // Build feature map for quick lookup
    let feature_map: std::collections::HashMap<Uuid, &FeatureMap> =
        features.iter().map(|(id, fm)| (*id, fm)).collect();

    // (a) Spatial convergence: sweep with simple pairwise check (O(N²))
    // For scale testing, a SpatialIndex could optimize this; for now, keep it simple and correct.
    for i in 0..players.len() {
        for j in (i + 1)..players.len() {
            let (a, pos_a, vel_a) = players[i];
            let (b, pos_b, vel_b) = players[j];

            // Skip if already strongly connected
            if graph.get_weight(a, b) >= hot_floor {
                continue;
            }

            let dx = pos_b.x - pos_a.x;
            let dy = pos_b.y - pos_a.y;
            let distance = (dx * dx + dy * dy).sqrt();

            // Check spatial proximity
            if distance <= screen_radius {
                let cs = closing_speed(pos_a, pos_b, vel_a, vel_b);
                if cs >= min_closing_speed {
                    candidates.push(ScreenCandidate {
                        a,
                        b,
                        pos_a,
                        pos_b,
                        vel_a,
                        vel_b,
                        history_weight: graph.get_weight(a, b),
                    });
                    continue;
                }
            }

            // (b) Graph adjacency: neighbors-of-neighbors
            // Build neighbor sets and check for mutual neighbors
            let mut neighbors_a = Vec::new();
            let mut neighbors_b = Vec::new();
            for (na, nb, weight) in graph.pairs() {
                if weight > 0.0 {
                    if na == a {
                        neighbors_a.push(nb);
                    } else if nb == a {
                        neighbors_a.push(na);
                    }
                    if nb == b {
                        neighbors_b.push(na);
                    } else if na == b {
                        neighbors_b.push(nb);
                    }
                }
            }

            for &neighbor_a in &neighbors_a {
                if neighbors_b.contains(&neighbor_a) {
                    candidates.push(ScreenCandidate {
                        a,
                        b,
                        pos_a,
                        pos_b,
                        vel_a,
                        vel_b,
                        history_weight: graph.get_weight(a, b),
                    });
                    break;
                }
            }
        }
    }

    // (c) Shared declared features from edge rules
    for (feature_name, _weight) in edge_rules {
        // Group entities by their value for this feature
        let mut feature_groups: std::collections::HashMap<String, Vec<Uuid>> =
            std::collections::HashMap::new();
        for (entity_id, fm) in &feature_map {
            if let Some(value) = fm.get(feature_name) {
                feature_groups
                    .entry(value.to_string())
                    .or_default()
                    .push(*entity_id);
            }
        }

        // For each group, emit pairs not already strongly connected
        for group in feature_groups.values() {
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    let a = group[i];
                    let b = group[j];

                    if graph.get_weight(a, b) >= hot_floor {
                        continue;
                    }

                    // Find positions in players array
                    if let (Some((_, pos_a, vel_a)), Some((_, pos_b, vel_b))) = (
                        players.iter().find(|(id, _, _)| *id == a),
                        players.iter().find(|(id, _, _)| *id == b),
                    ) {
                        candidates.push(ScreenCandidate {
                            a,
                            b,
                            pos_a: *pos_a,
                            pos_b: *pos_b,
                            vel_a: *vel_a,
                            vel_b: *vel_b,
                            history_weight: graph.get_weight(a, b),
                        });
                    }
                }
            }
        }
    }

    // Dedup by (min(a,b), max(a,b)) to avoid duplicates from multiple screening sources
    candidates.sort_by_key(|c| {
        let (a, b) = if c.a <= c.b { (c.a, c.b) } else { (c.b, c.a) };
        (a, b)
    });
    candidates.dedup_by_key(|c| {
        let (a, b) = if c.a <= c.b { (c.a, c.b) } else { (c.b, c.a) };
        (a, b)
    });

    candidates
}

/// Sweeps screened candidate pairs and returns promotions.
///
/// For each candidate, computes the predicted interaction probability using the predictor.
/// Only pairs whose predicted p > 0 are promoted (returned).
/// Features come from the lookup map; empty FeatureMap if entity has no features.
pub fn sweep_cold_pairs<P>(
    candidates: &[ScreenCandidate],
    predictor: &P,
    feature_lookup: &std::collections::HashMap<uuid::Uuid, FeatureMap>,
    horizon_secs: f64,
) -> Vec<Promotion>
where
    P: InteractionPredictor,
{
    let mut promotions = Vec::new();
    let empty_features = FeatureMap::new();

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

        // 3. Fetch features (or empty if not present)
        let features_a = feature_lookup.get(&candidate.a).unwrap_or(&empty_features);
        let features_b = feature_lookup.get(&candidate.b).unwrap_or(&empty_features);

        // 4. Build PairContext
        let ctx = PairContext {
            distance,
            closing_speed: cs,
            horizon_secs,
            history_weight: candidate.history_weight,
            features_a,
            features_b,
        };

        // 5. Predict
        let p = predictor.predict(&ctx);

        // 6. Promote if p > 0 (zero-cost-on-zero)
        if p > 0.0 {
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
    use crate::predictor::HeuristicPredictor;
    use uuid::Uuid;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn test_empty_input() {
        let predictor = HeuristicPredictor::default();
        let feature_lookup = std::collections::HashMap::new();
        let horizon_secs = 5.0;

        let result = sweep_cold_pairs(&[], &predictor, &feature_lookup, horizon_secs);

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
        // A candidate whose predicted p is very low should produce no output (zero-cost-on-zero).
        let predictor = HeuristicPredictor::default();
        let feature_lookup = std::collections::HashMap::new();
        let horizon_secs = 5.0;

        let candidate = ScreenCandidate {
            a: uuid(1),
            b: uuid(2),
            pos_a: Vec2::new(0.0, 0.0),
            pos_b: Vec2::new(100.0, 0.0),
            vel_a: Vec2::new(0.0, 0.0),
            vel_b: Vec2::new(0.0, 0.0),
            history_weight: 0.0,
        };

        let result = sweep_cold_pairs(&[candidate], &predictor, &feature_lookup, horizon_secs);

        // With no history and far apart (100 units), predicted p should be very low
        // so it won't produce a promotion
        assert!(
            result.is_empty() || result[0].p < 0.01,
            "far apart stationary pair should have very low p"
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
