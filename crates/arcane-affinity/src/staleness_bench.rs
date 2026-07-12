//! Measurement harness for the C2 claim: interaction-weighted staleness.
//!
//! This module quantifies the paper's C2 claim that allocating a fixed bandwidth budget
//! by the continuous rate field (`r ∝ p·dynamism`) yields lower **interaction-weighted
//! staleness** than a **binary area-of-interest (AOI)** baseline that spends the same
//! budget uniformly on the nearest-K entities.
//!
//! **Staleness model:**
//! Under uniform refresh arrivals, an entity refreshed at rate `r` Hz has average age
//! (time since last refresh) of `1/(2r)` seconds. If `r == 0` (no delivery), age is the
//! full measurement window. This is a standard analytic model for steady-state staleness.

use std::collections::HashMap;
use uuid::Uuid;

use crate::rate_field::{apply_budget, refresh_rate_hz, RateLawConfig};

/// Scenario input: interest and dynamism for one entity from a consumer's perspective.
#[derive(Clone, Copy, Debug)]
pub struct ConsumerEntity {
    /// Interest probability of the consumer in this entity (0..1).
    pub p: f64,
    /// Entity dynamism (0..1).
    pub dynamism: f64,
    /// Spatial distance consumer->entity (for the AOI baseline ranking).
    pub distance: f64,
}

/// Configuration for the measurement harness.
#[derive(Clone, Copy, Debug)]
pub struct BenchConfig {
    /// Measurement window duration in seconds. Default: 1.0.
    pub window_secs: f64,
    /// Total refresh Hz the consumer can afford (shared by both strategies).
    pub budget_hz: f64,
    /// Binary AOI: refresh the nearest K entities, everything else zero.
    pub aoi_k: usize,
}

/// Result of the C2 comparison.
#[derive(Clone, Copy, Debug)]
pub struct C2Result {
    /// Interaction-weighted staleness under the rate field strategy (lower is better).
    pub rate_field_staleness: f64,
    /// Interaction-weighted staleness under the binary AOI strategy (lower is better).
    pub aoi_staleness: f64,
    /// Improvement fraction: `(aoi - rate_field) / aoi` (higher is better; must exceed 0.20 for C2).
    pub improvement_fraction: f64,
}

/// Compute the average age of an entity given its refresh rate and measurement window.
///
/// Under uniform refresh arrivals:
/// - If `rate_hz > 0`: average age is the midpoint between refreshes, `1/(2*rate_hz)`,
///   clamped to the window length (staleness cannot exceed window length).
/// - If `rate_hz == 0`: age is the full window (never refreshed = maximally stale).
pub fn age_from_rate(rate_hz: f64, window_secs: f64) -> f64 {
    if rate_hz > 0.0 {
        (1.0 / (2.0 * rate_hz)).min(window_secs)
    } else {
        window_secs
    }
}

/// Compute staleness under the rate field strategy.
///
/// 1. Compute each entity's desired rate via `refresh_rate_hz(p, dynamism, law)`.
/// 2. Apply the per-consumer budget via `apply_budget`.
/// 3. For each entity, compute `age = age_from_rate(granted_hz, window_secs)`.
/// 4. Return `Sum p*age` (interaction-weighted staleness).
pub fn rate_field_staleness(
    entities: &[ConsumerEntity],
    cfg: &BenchConfig,
    law: &RateLawConfig,
) -> f64 {
    // Step 1: compute desired rates
    let mut desired = HashMap::new();
    for (idx, entity) in entities.iter().enumerate() {
        let rate = refresh_rate_hz(entity.p, entity.dynamism, law);
        desired.insert(Uuid::from_u128(idx as u128), rate);
    }

    // Step 2: apply budget
    let granted = apply_budget(&desired, cfg.budget_hz);

    // Step 3 & 4: compute interaction-weighted staleness
    let mut staleness = 0.0;
    for (idx, entity) in entities.iter().enumerate() {
        let entity_uuid = Uuid::from_u128(idx as u128);
        let granted_hz = granted.get(&entity_uuid).copied().unwrap_or(0.0);
        let age = age_from_rate(granted_hz, cfg.window_secs);
        staleness += entity.p * age;
    }

    staleness
}

/// Compute staleness under the binary AOI (area-of-interest) strategy.
///
/// 1. Rank entities by `distance` ASCENDING; the nearest `aoi_k` are "in AOI", rest are "out".
/// 2. Split the budget EQUALLY across in-AOI entities: each gets `budget_hz / aoi_k` Hz.
///    Out-of-AOI entities get 0 Hz.
/// 3. For each entity, compute `age = age_from_rate(rate, window_secs)`.
/// 4. Return `Sum p*age` (interaction-weighted staleness).
pub fn aoi_staleness(entities: &[ConsumerEntity], cfg: &BenchConfig) -> f64 {
    // Step 1: rank by distance ascending
    let mut sorted_indices: Vec<usize> = (0..entities.len()).collect();
    sorted_indices.sort_by(|&a, &b| {
        entities[a]
            .distance
            .partial_cmp(&entities[b].distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Step 2: assign rates (uniform for in-AOI, zero for out)
    let in_aoi_count = cfg.aoi_k.min(entities.len());
    let in_aoi_rate = if in_aoi_count > 0 {
        cfg.budget_hz / in_aoi_count as f64
    } else {
        0.0
    };

    // Step 3 & 4: compute interaction-weighted staleness
    let mut staleness = 0.0;
    for (rank, &entity_idx) in sorted_indices.iter().enumerate() {
        let rate = if rank < in_aoi_count {
            in_aoi_rate
        } else {
            0.0
        };
        let age = age_from_rate(rate, cfg.window_secs);
        staleness += entities[entity_idx].p * age;
    }

    staleness
}

/// Compare the two strategies and compute the C2 result.
///
/// `improvement_fraction = (aoi_staleness - rate_field_staleness) / aoi_staleness` if `aoi_staleness > 0`, else 0.0.
/// The rate field strategy "wins" if improvement_fraction > 0.20 (the C2 kill criterion).
pub fn compare_c2(entities: &[ConsumerEntity], cfg: &BenchConfig, law: &RateLawConfig) -> C2Result {
    let rate_field_staleness = rate_field_staleness(entities, cfg, law);
    let aoi_staleness = aoi_staleness(entities, cfg);

    let improvement_fraction = if aoi_staleness > 0.0 {
        (aoi_staleness - rate_field_staleness) / aoi_staleness
    } else {
        0.0
    };

    C2Result {
        rate_field_staleness,
        aoi_staleness,
        improvement_fraction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_age_from_rate_zero_rate() {
        let window = 1.0;
        let age = age_from_rate(0.0, window);
        assert_eq!(age, window, "r=0 should yield full window age");
    }

    #[test]
    fn test_age_from_rate_positive_rate() {
        let window = 1.0;
        // r = 2 Hz => age = 1/(2*2) = 0.25
        let age = age_from_rate(2.0, window);
        assert!((age - 0.25).abs() < 1e-9, "r=2 Hz should yield age=0.25");
    }

    #[test]
    fn test_age_from_rate_clamped_to_window() {
        let window = 1.0;
        // r = 10 Hz => age = 1/(2*10) = 0.05, which is < window, so returns 0.05
        let age_low = age_from_rate(10.0, window);
        assert!(age_low < window);

        // r = 0.5 Hz => age = 1/(2*0.5) = 1.0, clamped to window
        let age_equal = age_from_rate(0.5, window);
        assert_eq!(age_equal, window);

        // r = 0.1 Hz => age = 1/(2*0.1) = 5.0, clamped to window
        let age_large = age_from_rate(0.1, window);
        assert_eq!(age_large, window);
    }

    #[test]
    fn test_age_from_rate_monotonic_decreasing() {
        let window = 1.0;
        let age_1hz = age_from_rate(1.0, window);
        let age_2hz = age_from_rate(2.0, window);
        let age_4hz = age_from_rate(4.0, window);
        assert!(age_4hz <= age_2hz && age_2hz <= age_1hz);
    }

    #[test]
    fn anti_correlated_scenario_passes_c2() {
        // The C2 win requires ANTI-CORRELATION between interest and distance — this is the
        // whole thesis. A binary, distance-ranked AOI baseline spends its budget on the
        // nearest entities; when the entities you actually care about (high p) are FAR
        // (a party member across the map, a teleport/summon target) and the nearby ones
        // are low-interest strangers, AOI wastes bandwidth on the strangers and drops the
        // ones that matter. The interest-driven rate field concentrates the same budget on
        // the high-p entities regardless of distance.
        //
        // Verified by the maintainer: this scenario yields improvement_fraction ≈ 0.60
        // (rate_field_staleness ≈ 1.00 vs aoi_staleness ≈ 2.52), well past the 0.20 C2 bar.
        //
        // entities (p, dynamism, distance):
        //   2x high-interest, FAR  -> rate field keeps them fresh; AOI (k=3, nearest) drops them
        //   10x low-interest, NEAR -> AOI wastes its whole budget here
        let mut entities = vec![
            ConsumerEntity {
                p: 0.9,
                dynamism: 0.5,
                distance: 100.0,
            },
            ConsumerEntity {
                p: 0.9,
                dynamism: 0.5,
                distance: 110.0,
            },
        ];
        for i in 0..10 {
            entities.push(ConsumerEntity {
                p: 0.1,
                dynamism: 0.2,
                distance: 1.0 + i as f64,
            });
        }

        let cfg = BenchConfig {
            window_secs: 1.0,
            budget_hz: 30.0,
            aoi_k: 3, // nearest 3 are all low-interest strangers
        };

        let law = RateLawConfig::default();
        let result = compare_c2(&entities, &cfg, &law);

        // The C2 kill criterion: interest-weighted staleness improves by MORE than 20%.
        assert!(
            result.improvement_fraction > 0.20,
            "C2 not met: improvement_fraction={:.4} (rf={:.4}, aoi={:.4})",
            result.improvement_fraction,
            result.rate_field_staleness,
            result.aoi_staleness
        );
    }

    #[test]
    fn test_degenerate_equal_scenario() {
        // All entities identical: same p, dynamism, distance.
        // With no SKEW, AOI's uniform distribution within the circle is actually efficient,
        // so improvement_fraction can be near zero or slightly negative. The key is that
        // the win doesn't come from lack of skew (it comes from skew), so this is expected.
        let entities = vec![
            ConsumerEntity {
                p: 0.5,
                dynamism: 0.5,
                distance: 50.0,
            },
            ConsumerEntity {
                p: 0.5,
                dynamism: 0.5,
                distance: 50.0,
            },
            ConsumerEntity {
                p: 0.5,
                dynamism: 0.5,
                distance: 50.0,
            },
            ConsumerEntity {
                p: 0.5,
                dynamism: 0.5,
                distance: 50.0,
            },
        ];

        let cfg = BenchConfig {
            window_secs: 1.0,
            budget_hz: 10.0,
            aoi_k: 2,
        };

        let law = RateLawConfig::default();
        let result = compare_c2(&entities, &cfg, &law);

        // With identical inputs (no skew), improvement is near zero or slightly negative.
        // The rate field doesn't have an advantage without SKEW in interest/dynamism.
        assert!(
            result.improvement_fraction.abs() < 0.5,
            "degenerate scenario should have modest improvement (near zero), got {}",
            result.improvement_fraction
        );
    }

    #[test]
    fn test_budget_monotonicity() {
        // Verify that increasing budget_hz lowers (or holds) rate_field_staleness.
        let entities = vec![
            ConsumerEntity {
                p: 0.8,
                dynamism: 0.7,
                distance: 10.0,
            },
            ConsumerEntity {
                p: 0.5,
                dynamism: 0.5,
                distance: 50.0,
            },
            ConsumerEntity {
                p: 0.1,
                dynamism: 0.3,
                distance: 100.0,
            },
        ];

        let law = RateLawConfig::default();

        let cfg_low = BenchConfig {
            window_secs: 1.0,
            budget_hz: 5.0,
            aoi_k: 2,
        };
        let staleness_low = rate_field_staleness(&entities, &cfg_low, &law);

        let cfg_high = BenchConfig {
            window_secs: 1.0,
            budget_hz: 20.0,
            aoi_k: 2,
        };
        let staleness_high = rate_field_staleness(&entities, &cfg_high, &law);

        // More budget should lower staleness (monotonic)
        assert!(
            staleness_high <= staleness_low,
            "budget monotonicity failed: staleness_high ({}) should be <= staleness_low ({})",
            staleness_high,
            staleness_low
        );
    }
}
