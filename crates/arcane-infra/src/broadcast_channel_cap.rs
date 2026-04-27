//! Broadcast channel buffer cap, sourced from the environment.
//!
//! The cluster's per-tick `PreEncodedTick` is fanned out to all subscriber
//! tasks via a single `tokio::sync::broadcast::channel`. The channel buffer
//! cap is the deepest backlog the slowest subscriber can fall behind by
//! before the channel fires `Lagged` and drops the oldest frames.
//!
//! ## Why it's tunable
//!
//! Empirically (see `docs/BENCHMARK_JOURNAL.md`, 2026-04-27 entry), at the
//! 4,750-CCU realistic-state ceiling on c7i.2xlarge clusters_4 + dead
//! reckoning, cluster CPU ran at 18% of the 30 Hz tick budget and NIC was
//! at ~80% of c7i.2xlarge sustained throughput — *not* multi-x oversubscribed.
//! Yet `broadcast_lagged_frames` kept firing in the hundreds-of-thousands.
//! That means the channel cap was the binding constraint, not CPU and not
//! NIC. The fix lets operators raise it past the prior hardcoded 256
//! without rebuilding.
//!
//! ## Memory cost
//!
//! Each slot holds an `Arc<PreEncodedTick>`. The pointer itself is 8 B; the
//! pointee carries `entity_chunks: Vec<Arc<Vec<u8>>>` whose total payload
//! varies with cluster entity count. At 1 K entities × ~150 B per chunk
//! that's ~150 KB per `PreEncodedTick`. A 2048-slot buffer worst-case is
//! ~300 MB per cluster — comfortably within the 16 GB c7i.2xlarge budget,
//! but operators should size proportionally to their own clusters' entity
//! counts and per-entity wire size. The clamp range below caps any single
//! misconfiguration to a sane bound.

const ENV_VAR: &str = "ARCANE_BROADCAST_CHANNEL_CAP";

/// Default buffer cap. Larger than the prior hardcoded 256 because the
/// 2026-04-27 measurements found 256 was the binding constraint at the
/// headline 30 Hz / 100 ms tier on commodity hardware. 2048 ≈ 70 sec of
/// 30 Hz cluster-tick history; chosen to give substantial headroom
/// without runaway memory growth at typical entity counts.
const DEFAULT_CAP: usize = 2048;

/// Lower bound. Below this the channel basically can't absorb any
/// scheduler jitter; tokio's broadcast docs require a positive non-zero
/// capacity anyway.
const MIN_CAP: usize = 64;

/// Upper bound. At ~150 KB per `PreEncodedTick` and 65536 slots, worst
/// case is ~10 GB per cluster. Above this and operators are almost
/// certainly mis-tuning; force them to think.
const MAX_CAP: usize = 65_536;

/// Pure resolver: given the raw env value (Some = env var was set, None =
/// unset), return the resolved cap. Extracted from the env-reading shell
/// so tests can exercise every branch without mutating process env state
/// (which races across the test runner's threads).
fn resolve(raw: Option<&str>) -> usize {
    raw.and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_CAP)
        .clamp(MIN_CAP, MAX_CAP)
}

/// Resolved broadcast-channel buffer cap. Cheap to call repeatedly — it
/// re-reads the env var each time — but typical callers resolve once at
/// channel construction.
pub fn broadcast_channel_cap() -> usize {
    resolve(std::env::var(ENV_VAR).ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_unset() {
        assert_eq!(resolve(None), DEFAULT_CAP);
    }

    #[test]
    fn parses_env_value() {
        assert_eq!(resolve(Some("4096")), 4096);
        assert_eq!(resolve(Some("256")), 256);
    }

    #[test]
    fn clamps_below_min() {
        assert_eq!(resolve(Some("16")), MIN_CAP);
    }

    #[test]
    fn clamps_above_max() {
        assert_eq!(resolve(Some("1000000")), MAX_CAP);
    }

    #[test]
    fn falls_back_on_unparseable_value() {
        assert_eq!(resolve(Some("not-a-number")), DEFAULT_CAP);
    }

    #[test]
    fn falls_back_on_zero() {
        // 0 parses fine but is an invalid channel capacity (tokio
        // requires > 0). Clamp catches it via MIN_CAP.
        assert_eq!(resolve(Some("0")), MIN_CAP);
    }
}
