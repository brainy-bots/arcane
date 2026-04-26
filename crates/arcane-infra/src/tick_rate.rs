//! Resolved cluster simulation tick rate, sourced from the environment.
//!
//! Single source of truth for `cluster_runner` (drives the tick loop) and
//! `spacetimedb_persist` (computes the persist cadence in ticks). Reading the
//! env var in one place keeps the two from drifting if a future change moves
//! the default.
//!
//! The clamp at [5, 128] Hz is wide enough to span the MMO band (5–30 Hz) and
//! the shooter/competitive band (30–128 Hz). Outside that range, callers
//! likely typo'd the env var and should see the silently-clamped value rather
//! than e.g. a 1 Hz tick that produces "the cluster looks alive but no entity
//! has moved" pathology.
//!
//! Default 20 Hz preserves the historical baseline: prior tick-rate constants
//! were `const TICK_RATE_HZ: u64 = 20`, and configs/tfvars in callers that
//! don't set the env var should see no behavioral change.

const ENV_VAR: &str = "BENCHMARK_TICK_RATE_HZ";
const DEFAULT_HZ: u64 = 20;
const MIN_HZ: u64 = 5;
const MAX_HZ: u64 = 128;

/// Pure resolver: given the raw env value (Some = env var was set, None =
/// unset), returns the resolved tick rate. Extracted from the env-reading
/// shell so tests can exercise every branch without mutating process env
/// (which races across the test runner's threads).
fn resolve(raw: Option<&str>) -> u64 {
    raw.and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_HZ)
        .clamp(MIN_HZ, MAX_HZ)
}

/// Resolved cluster tick rate in Hz, clamped to [MIN_HZ, MAX_HZ]. Cheap to
/// call repeatedly — it re-reads the env var each time — but typical callers
/// resolve it once at startup.
pub fn tick_rate_hz() -> u64 {
    resolve(std::env::var(ENV_VAR).ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_20_when_unset() {
        assert_eq!(resolve(None), DEFAULT_HZ);
    }

    #[test]
    fn parses_env_value() {
        assert_eq!(resolve(Some("30")), 30);
        assert_eq!(resolve(Some("60")), 60);
    }

    #[test]
    fn clamps_below_min() {
        assert_eq!(resolve(Some("1")), MIN_HZ);
    }

    #[test]
    fn clamps_above_max() {
        assert_eq!(resolve(Some("256")), MAX_HZ);
    }

    #[test]
    fn falls_back_on_unparseable_value() {
        assert_eq!(resolve(Some("not-a-number")), DEFAULT_HZ);
    }
}
