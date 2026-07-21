use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Coarse spectrum of refresh rates: full-fidelity, reduced, or no delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Budget-aware rate allocation: the spectrum curve is the ORDERING, the
/// budget only sets its SCALE (water-filling). `rate_i = max_hz * min(1,
/// k * s_i)` with k chosen so the total fits `budget_hz`.
///
/// - Budget DISABLED (`budget_hz` infinite, the default): k = 1 — the
///   allocation is exactly the plain spectrum curve (`refresh_rate_hz`).
///   No boost, no squeeze; today's behavior, bit for bit.
/// - Headroom (finite budget covering everyone at max_hz): k is unbounded —
///   every entity in range replicates at FULL speed. "Only two in range ->
///   both at full rate": configuring a real budget states that unused
///   capacity SHOULD be spent, so the curve is boosted to saturation.
/// - Saturation: k shrinks and rates degrade CONTINUOUSLY, ordered by s.
///   Higher s always gets >= the rate of lower s; adjacent s get adjacent
///   rates (no full-or-zero cliff at the budget line — a greedy cut would
///   re-introduce binary attention exactly at the stress boundary).
/// - `zero_floor` applies to the SCALED signal: under load the tail drops
///   below the floor and vanishes — the attention horizon contracts when
///   the system is busy and expands when idle, emergently.
///
/// Inputs are raw spectrum signals `s = clamp(p * dynamism)` per entity
/// (NOT Hz). Pure and deterministic: any stateless router worker computes
/// the same allocation from the same doc. Forced (gate) entities must be
/// handled OUTSIDE this function — they are correctness traffic, never
/// budgeted.
pub fn allocate_rates(
    signals: &HashMap<Uuid, f64>,
    budget_hz: f64,
    config: &RateLawConfig,
) -> HashMap<Uuid, f64> {
    let mut granted: HashMap<Uuid, f64> = HashMap::new();
    // Positive-signal entities, sorted by s DESC then Uuid ASC (determinism).
    let mut active: Vec<(Uuid, f64)> = signals
        .iter()
        .filter(|(_, s)| **s > 0.0)
        .map(|(id, s)| (*id, s.clamp(0.0, 1.0)))
        .collect();
    for (id, s) in signals {
        if *s <= 0.0 {
            granted.insert(*id, 0.0);
        }
    }
    if active.is_empty() {
        return granted;
    }
    active.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let n = active.len();
    let full_total = config.max_hz * n as f64;
    // k = the water level. Infinite budget = mechanism DISABLED = plain
    // curve (k=1). A finite budget covering everyone at max => k unbounded
    // => all full rate (spend the capacity that was declared available).
    let k = if !budget_hz.is_finite() {
        1.0
    } else if budget_hz >= full_total {
        f64::INFINITY
    } else if budget_hz <= 0.0 {
        0.0
    } else {
        // Find m = number of SATURATED entities (top of the sort). For a
        // candidate m: the unsaturated tail shares the remaining budget in
        // proportion to s, k = (budget - m*max) / (max * sum_tail_s).
        // Consistency: the first unsaturated entity must really be under
        // the cap (k*s < 1) and the last saturated one over it (k*s >= 1).
        // k is monotone in m, so exactly one m is consistent; n is small
        // (one doc's candidates), the linear scan is fine.
        let mut chosen = 0.0;
        for m in 0..n {
            let tail_s: f64 = active[m..].iter().map(|(_, s)| s).sum();
            if tail_s <= 0.0 {
                break;
            }
            let k = (budget_hz - config.max_hz * m as f64) / (config.max_hz * tail_s);
            if k <= 0.0 {
                break;
            }
            let first_unsat_ok = k * active[m].1 < 1.0;
            let last_sat_ok = m == 0 || k * active[m - 1].1 >= 1.0;
            if first_unsat_ok && last_sat_ok {
                chosen = k;
                break;
            }
        }
        chosen
    };

    for (id, s) in active {
        let s_eff = (k * s).min(1.0);
        let rate = if s_eff < config.zero_floor {
            0.0
        } else {
            config.max_hz * s_eff
        };
        granted.insert(id, rate);
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

    fn sig(pairs: &[(u8, f64)]) -> HashMap<Uuid, f64> {
        pairs
            .iter()
            .map(|(n, s)| (Uuid::from_bytes([*n; 16]), *s))
            .collect()
    }

    #[test]
    fn test_allocate_headroom_everyone_full() {
        // The founder's case: only two in range and a REAL budget with
        // headroom -> BOTH at full speed, regardless of their p ordering.
        let config = RateLawConfig::default();
        let signals = sig(&[(1, 0.9), (2, 0.3)]);
        let granted = allocate_rates(&signals, config.max_hz * 2.0, &config);
        assert_eq!(granted[&Uuid::from_bytes([1; 16])], config.max_hz);
        assert_eq!(granted[&Uuid::from_bytes([2; 16])], config.max_hz);
    }

    #[test]
    fn test_allocate_disabled_is_plain_curve() {
        // budget = INFINITY means the mechanism is OFF: allocation must be
        // exactly refresh_rate_hz for every entity — including the zero
        // floor (a p~0 straggler must NOT be boosted to full rate just
        // because no budget was configured).
        let config = RateLawConfig::default();
        let signals = sig(&[(1, 0.9), (2, 0.3), (3, 0.01)]);
        let granted = allocate_rates(&signals, f64::INFINITY, &config);
        for (id, s) in &signals {
            assert_eq!(
                granted[id],
                refresh_rate_hz(*s, 1.0, &config),
                "disabled budget must equal the plain curve at s={s}"
            );
        }
        assert_eq!(granted[&Uuid::from_bytes([3; 16])], 0.0);
    }

    #[test]
    fn test_allocate_saturation_monotone_no_cliff() {
        let config = RateLawConfig::default();
        let signals = sig(&[(1, 0.9), (2, 0.6), (3, 0.5), (4, 0.4)]);
        // Budget = half of everyone-full.
        let budget = config.max_hz * 2.0;
        let granted = allocate_rates(&signals, budget, &config);
        let g = |n: u8| granted[&Uuid::from_bytes([n; 16])];
        // Total fits.
        let total: f64 = granted.values().sum();
        assert!(total <= budget + 1e-9, "total {total} > budget {budget}");
        // Monotone in s.
        assert!(g(1) >= g(2) && g(2) >= g(3) && g(3) >= g(4));
        // No cliff: adjacent s values (0.5 vs 0.4) must get rates in
        // roughly their signal ratio, NOT full-vs-zero.
        assert!(g(4) > 0.0, "adjacent-s entity fell off a cliff");
        assert!(
            g(3) / g(4) < 2.0,
            "rate gap {} vs {} disproportionate",
            g(3),
            g(4)
        );
    }

    #[test]
    fn test_allocate_water_level_saturates_top() {
        let config = RateLawConfig::default();
        // One dominant, two weak; budget affords ~1.5 full rates. The
        // dominant SATURATES at max_hz (never exceeds it), the tail shares
        // the remainder proportionally.
        let signals = sig(&[(1, 1.0), (2, 0.2), (3, 0.1)]);
        let budget = config.max_hz * 1.5;
        let granted = allocate_rates(&signals, budget, &config);
        let g = |n: u8| granted[&Uuid::from_bytes([n; 16])];
        assert!((g(1) - config.max_hz).abs() < 1e-9, "top should saturate");
        assert!(g(2) > g(3) && g(3) > 0.0);
        let tail_ratio = g(2) / g(3);
        assert!(
            (tail_ratio - 2.0).abs() < 0.2,
            "tail not proportional: {tail_ratio}"
        );
    }

    #[test]
    fn test_allocate_floor_contracts_horizon_under_load() {
        let config = RateLawConfig::default();
        // Weak signal above the floor unloaded, pushed below it when the
        // budget squeezes k: horizon contracts under load.
        let signals_unloaded = sig(&[(1, 0.05)]);
        let g_unloaded = allocate_rates(&signals_unloaded, config.max_hz * 100.0, &config);
        assert!(g_unloaded[&Uuid::from_bytes([1; 16])] > 0.0);

        let mut signals_loaded = sig(&[(1, 0.05)]);
        for n in 2..=20u8 {
            signals_loaded.insert(Uuid::from_bytes([n; 16]), 1.0);
        }
        // Budget: enough for ~a quarter of the strong entities. k =
        // (5*max)/(max*19.05) = 0.26; weak s_eff = 0.013 < floor 0.02.
        let g_loaded = allocate_rates(&signals_loaded, config.max_hz * 5.0, &config);
        assert_eq!(
            g_loaded[&Uuid::from_bytes([1; 16])],
            0.0,
            "weak signal should drop below the floor under load"
        );
    }

    #[test]
    fn test_allocate_zero_budget_and_zero_signal() {
        let config = RateLawConfig::default();
        let signals = sig(&[(1, 0.9), (2, 0.0)]);
        let granted = allocate_rates(&signals, 0.0, &config);
        assert_eq!(granted[&Uuid::from_bytes([1; 16])], 0.0);
        assert_eq!(granted[&Uuid::from_bytes([2; 16])], 0.0);
    }

    #[test]
    fn test_allocate_deterministic() {
        let config = RateLawConfig::default();
        let signals = sig(&[(3, 0.5), (1, 0.5), (2, 0.5)]);
        let a = allocate_rates(&signals, config.max_hz * 1.0, &config);
        let b = allocate_rates(&signals, config.max_hz * 1.0, &config);
        assert_eq!(a, b);
        // Equal signals, equal rates: continuous sharing has no tie-break
        // winners (unlike a greedy cut).
        let vals: Vec<f64> = a.values().copied().collect();
        assert!((vals[0] - vals[1]).abs() < 1e-9 && (vals[1] - vals[2]).abs() < 1e-9);
    }
}
