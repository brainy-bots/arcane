use std::collections::HashMap;
use uuid::Uuid;

/// Coarse spectrum of refresh rates: full-fidelity, reduced, or no delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateTier {
    Full,
    Low,
    Zero,
}

/// Configuration for the rate law: refresh rate cap, truncation floor, and tier boundaries.
#[derive(Clone, Copy, Debug)]
pub struct RateLawConfig {
    /// Full-fidelity refresh rate cap (Hz). Default: 30.0.
    pub max_hz: f64,
    /// Below this threshold, rate is truncated to zero. Default: 0.02.
    pub zero_floor: f64,
    /// Normalized rate below this threshold (but >= zero_floor) maps to Low tier. Default: 0.5.
    pub low_threshold: f64,
}

impl Default for RateLawConfig {
    fn default() -> Self {
        Self {
            max_hz: 30.0,
            zero_floor: 0.02,
            low_threshold: 0.5,
        }
    }
}

/// Continuous refresh rate in Hz for a single (entity, consumer) pair.
/// Applies interest-weighted signal (p * dynamism), clamped to [0,1], then truncates below zero_floor.
pub fn refresh_rate_hz(p: f64, dynamism: f64, config: &RateLawConfig) -> f64 {
    let s = (p * dynamism).clamp(0.0, 1.0);
    if s < config.zero_floor {
        return 0.0;
    }
    config.max_hz * s
}

/// Map a (p, dynamism) pair to a coarse tier.
/// Returns Zero if below zero_floor, Low if between zero_floor and low_threshold, else Full.
pub fn rate_tier(p: f64, dynamism: f64, config: &RateLawConfig) -> RateTier {
    let s = (p * dynamism).clamp(0.0, 1.0);
    if s < config.zero_floor {
        RateTier::Zero
    } else if s < config.low_threshold {
        RateTier::Low
    } else {
        RateTier::Full
    }
}

/// Apply per-consumer budget by greedily granting full rate to highest-interest entities.
/// When sum(desired) exceeds budget, demote least-critical entities to zero.
/// Returns the granted rate per entity (same keys as input).
/// Ties are broken by Uuid ascending for determinism.
pub fn apply_budget(desired: &HashMap<Uuid, f64>, budget_hz: f64) -> HashMap<Uuid, f64> {
    let sum_desired: f64 = desired.values().sum();

    if sum_desired <= budget_hz {
        return desired.clone();
    }

    // Sort by desired rate descending, then by Uuid ascending for tie-break.
    let mut sorted: Vec<_> = desired.iter().collect();
    sorted.sort_by(|a, b| {
        b.1.partial_cmp(a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });

    let mut granted = HashMap::new();
    let mut running_total = 0.0;

    for (entity, rate) in sorted {
        if running_total + rate <= budget_hz {
            granted.insert(*entity, *rate);
            running_total += rate;
        } else {
            granted.insert(*entity, 0.0);
        }
    }

    // Ensure all input keys are present.
    for entity in desired.keys() {
        granted.entry(*entity).or_insert(0.0);
    }

    granted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monotonic_interest() {
        let config = RateLawConfig::default();
        let rate_low = refresh_rate_hz(0.3, 0.5, &config);
        let rate_high = refresh_rate_hz(0.5, 0.5, &config);
        assert!(rate_high >= rate_low);
    }

    #[test]
    fn test_monotonic_dynamism() {
        let config = RateLawConfig::default();
        let rate_low = refresh_rate_hz(0.5, 0.3, &config);
        let rate_high = refresh_rate_hz(0.5, 0.5, &config);
        assert!(rate_high >= rate_low);
    }

    #[test]
    fn test_truncation_at_zero_floor() {
        let config = RateLawConfig::default();
        let s = 0.01; // Below zero_floor (0.02)
        let rate = refresh_rate_hz(s, 1.0, &config);
        assert_eq!(rate, 0.0);

        let tier = rate_tier(s, 1.0, &config);
        assert_eq!(tier, RateTier::Zero);
    }

    #[test]
    fn test_cap_at_max_hz() {
        let config = RateLawConfig::default();
        let rate = refresh_rate_hz(1.0, 1.0, &config);
        assert_eq!(rate, config.max_hz);

        let tier = rate_tier(1.0, 1.0, &config);
        assert_eq!(tier, RateTier::Full);
    }

    #[test]
    fn test_low_tier() {
        let config = RateLawConfig::default();
        // s between zero_floor (0.02) and low_threshold (0.5)
        let s = 0.3;
        let rate = refresh_rate_hz(s, 1.0, &config);
        assert!(rate > 0.0);
        assert!(rate < config.max_hz * config.low_threshold);

        let tier = rate_tier(s, 1.0, &config);
        assert_eq!(tier, RateTier::Low);
    }

    #[test]
    fn test_budget_no_pressure() {
        let mut desired = HashMap::new();
        desired.insert(Uuid::nil(), 10.0);
        desired.insert(Uuid::max(), 5.0);

        let budget = 20.0;
        let granted = apply_budget(&desired, budget);

        assert_eq!(granted[&Uuid::nil()], 10.0);
        assert_eq!(granted[&Uuid::max()], 5.0);
    }

    #[test]
    fn test_budget_demotion() {
        let e1 = Uuid::nil();
        let e2 = Uuid::max();
        let e3 = Uuid::new_v4();

        let mut desired = HashMap::new();
        desired.insert(e1, 5.0);
        desired.insert(e2, 3.0);
        desired.insert(e3, 2.0);

        // Budget only affords the top two (5.0 + 3.0 = 8.0)
        let budget = 8.0;
        let granted = apply_budget(&desired, budget);

        assert_eq!(granted[&e1], 5.0);
        assert_eq!(granted[&e2], 3.0);
        assert_eq!(granted[&e3], 0.0);
    }

    #[test]
    fn test_budget_demotion_determinism() {
        // Three entities with the same desired rate; ties break by Uuid ascending.
        let e1 = "00000000-0000-0000-0000-000000000001"
            .parse::<Uuid>()
            .unwrap();
        let e2 = "00000000-0000-0000-0000-000000000002"
            .parse::<Uuid>()
            .unwrap();
        let e3 = "00000000-0000-0000-0000-000000000003"
            .parse::<Uuid>()
            .unwrap();

        let mut desired = HashMap::new();
        desired.insert(e1, 2.0);
        desired.insert(e2, 2.0);
        desired.insert(e3, 2.0);

        // Budget affords only two full rates.
        let budget = 4.0;
        let granted = apply_budget(&desired, budget);

        // The two with lowest Uuid should be granted; e3 demoted.
        assert_eq!(granted[&e1], 2.0);
        assert_eq!(granted[&e2], 2.0);
        assert_eq!(granted[&e3], 0.0);
    }

    #[test]
    fn test_budget_zero() {
        let mut desired = HashMap::new();
        desired.insert(Uuid::nil(), 5.0);
        desired.insert(Uuid::max(), 3.0);

        let granted = apply_budget(&desired, 0.0);

        assert_eq!(granted[&Uuid::nil()], 0.0);
        assert_eq!(granted[&Uuid::max()], 0.0);
    }
}
