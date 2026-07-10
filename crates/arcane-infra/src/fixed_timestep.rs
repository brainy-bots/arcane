//! Fixed-timestep accumulator (Gaffer-on-Games pattern).
//!
//! A variable-rate driver feeds elapsed wall-clock time and receives how many
//! fixed sim-steps are due this frame. Deterministic and testable without live
//! wall-clock, as the core logic operates purely on durations.

use std::time::{Duration, Instant};

/// Fixed-timestep accumulator (Gaffer-on-Games pattern). A variable-rate driver
/// feeds it elapsed wall-clock and it returns how many fixed sim-steps are due
/// this frame.
#[derive(Debug)]
pub struct FixedTimestep {
    interval: Duration,
    accumulator: Duration,
    last: Option<Instant>,
}

impl FixedTimestep {
    /// Interval = 1000 / tick_rate_hz ms (mirrors run_node_loop's integer-ms
    /// interval exactly).
    pub fn from_tick_rate() -> Self {
        Self::with_interval(Duration::from_millis(
            1000 / crate::tick_rate::tick_rate_hz(),
        ))
    }

    /// Create a fixed-timestep accumulator with a given interval.
    pub fn with_interval(interval: Duration) -> Self {
        Self {
            interval,
            accumulator: Duration::ZERO,
            last: None,
        }
    }

    /// The fixed sim-tick interval.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Add `elapsed` to the accumulator and return how many whole fixed steps
    /// are now due, draining the accumulator by that many intervals. Pure +
    /// deterministic — unit-test this directly with `Duration` values (no
    /// wall-clock dependence). Guards against a zero interval.
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        if self.interval.is_zero() {
            return 0;
        }
        self.accumulator += elapsed;
        let mut steps = 0u32;
        while self.accumulator >= self.interval {
            self.accumulator -= self.interval;
            steps += 1;
        }
        steps
    }

    /// Convenience for wall-clock drivers: computes elapsed since the last call
    /// (0 on first call) and delegates to `advance`. Built on `Instant`, so
    /// cover it lightly; put the real assertions on `advance`.
    pub fn steps_due(&mut self, now: Instant) -> u32 {
        let elapsed = match self.last {
            Some(prev) => now.saturating_duration_since(prev),
            None => Duration::ZERO,
        };
        self.last = Some(now);
        self.advance(elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_less_than_interval_returns_zero() {
        let interval = Duration::from_millis(50);
        let mut acc = FixedTimestep::with_interval(interval);
        assert_eq!(acc.advance(Duration::from_millis(30)), 0);
    }

    #[test]
    fn advance_less_than_interval_carries_remainder() {
        let interval = Duration::from_millis(50);
        let mut acc = FixedTimestep::with_interval(interval);
        assert_eq!(acc.advance(Duration::from_millis(30)), 0);
        assert_eq!(acc.advance(Duration::from_millis(30)), 1);
    }

    #[test]
    fn advance_exactly_interval_returns_one() {
        let interval = Duration::from_millis(50);
        let mut acc = FixedTimestep::with_interval(interval);
        assert_eq!(acc.advance(Duration::from_millis(50)), 1);
    }

    #[test]
    fn advance_multiple_intervals_returns_correct_count() {
        let interval = Duration::from_millis(50);
        let mut acc = FixedTimestep::with_interval(interval);
        assert_eq!(acc.advance(Duration::from_millis(175)), 3);
    }

    #[test]
    fn advance_multiple_intervals_with_remainder() {
        let interval = Duration::from_millis(50);
        let mut acc = FixedTimestep::with_interval(interval);
        assert_eq!(acc.advance(Duration::from_millis(175)), 3);
        assert_eq!(acc.advance(Duration::from_millis(0)), 0);
        assert_eq!(acc.advance(Duration::from_millis(25)), 1);
    }

    #[test]
    fn advance_zero_interval_guards_and_returns_zero() {
        let mut acc = FixedTimestep::with_interval(Duration::ZERO);
        assert_eq!(acc.advance(Duration::from_millis(100)), 0);
        assert_eq!(acc.advance(Duration::from_millis(100)), 0);
    }

    #[test]
    fn from_tick_rate_matches_interval() {
        let acc = FixedTimestep::from_tick_rate();
        let expected = Duration::from_millis(1000 / crate::tick_rate::tick_rate_hz());
        assert_eq!(acc.interval(), expected);
    }

    #[test]
    fn steps_due_first_call_returns_zero() {
        let interval = Duration::from_millis(50);
        let mut acc = FixedTimestep::with_interval(interval);
        let now = Instant::now();
        assert_eq!(acc.steps_due(now), 0);
    }

    #[test]
    fn steps_due_computes_elapsed() {
        let interval = Duration::from_millis(50);
        let mut acc = FixedTimestep::with_interval(interval);
        let now1 = Instant::now();
        acc.steps_due(now1);
        std::thread::sleep(Duration::from_millis(60));
        let now2 = Instant::now();
        let steps = acc.steps_due(now2);
        assert!(steps >= 1, "Expected at least 1 step");
    }
}
