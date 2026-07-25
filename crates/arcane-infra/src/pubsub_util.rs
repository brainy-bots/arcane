//! Resilient Redis pub/sub subscription loop, shared by every subscriber
//! thread in this crate (arcane#204 hardening).
//!
//! The original subscribers all had the same shape — connect once, subscribe,
//! `loop { get_message() }`, and on ANY error: log + `break`. That kills the
//! thread permanently on the first dropped connection, and the channel it fed
//! (neighbor deltas, node inbox frames, forwarded inputs, ownership flips,
//! physics events) silently goes dark until process restart — the exact
//! "silently stops delivering" failure mode #204 was opened about. (The
//! specific incident there turned out to be a crashed publisher, but the
//! subscriber fragility was real and is fixed here.)
//!
//! This helper wraps the whole lifecycle in an OUTER reconnect loop with
//! exponential backoff (250ms → 5s cap, reset on successful subscribe).
//! Redis pub/sub is fire-and-forget, so messages published during a gap are
//! lost — that is acceptable by design: every channel this crate subscribes
//! to is either refreshed continuously (state frames at tick rate, resync
//! cadence) or re-derived by the next control cycle (flips, forwarded
//! inputs re-sent at client rate).
//!
//! The handler returns `ControlFlow`: `Break` means the consuming side is
//! gone (its mpsc receiver dropped) and the thread should exit for real.

use std::ops::ControlFlow;
use std::thread;
use std::time::Duration;

/// Minimum backoff after a failed connect/subscribe or a dropped connection.
const BACKOFF_MIN: Duration = Duration::from_millis(250);
/// Backoff cap.
const BACKOFF_MAX: Duration = Duration::from_secs(5);

/// Next backoff step: double, capped.
fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(BACKOFF_MAX)
}

/// Spawn a named subscriber thread that stays subscribed to `topics` on
/// `redis_url` forever, reconnecting with backoff on any error. Each payload
/// is passed to `on_payload`; return `ControlFlow::Break(())` to stop the
/// thread permanently (consumer gone).
pub fn spawn_resilient_subscriber(
    name: &'static str,
    redis_url: String,
    topics: Vec<String>,
    mut on_payload: impl FnMut(String) -> ControlFlow<()> + Send + 'static,
) {
    if topics.is_empty() {
        return;
    }
    thread::spawn(move || {
        let mut backoff = BACKOFF_MIN;
        loop {
            // One connection attempt + subscribe + drain session.
            match subscribe_session(name, &redis_url, &topics, &mut on_payload) {
                SessionEnd::ConsumerGone => return,
                SessionEnd::ConnectionLost => {
                    eprintln!("{name}: reconnecting in {backoff:?}");
                    thread::sleep(backoff);
                    backoff = next_backoff(backoff);
                }
                SessionEnd::Subscribed => {
                    // We had a working session (delivered at least the
                    // subscribe): reset backoff before the next attempt.
                    backoff = BACKOFF_MIN;
                    eprintln!("{name}: connection dropped; reconnecting in {backoff:?}");
                    thread::sleep(backoff);
                }
            }
        }
    });
}

enum SessionEnd {
    /// Handler said stop: exit the thread.
    ConsumerGone,
    /// Could not connect/subscribe at all: back off (growing).
    ConnectionLost,
    /// A live session dropped after successful subscribe: back off (reset).
    Subscribed,
}

fn subscribe_session(
    name: &str,
    redis_url: &str,
    topics: &[String],
    on_payload: &mut impl FnMut(String) -> ControlFlow<()>,
) -> SessionEnd {
    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{name}: Redis open failed: {e}");
            return SessionEnd::ConnectionLost;
        }
    };
    let mut conn = match client.get_connection() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{name}: Redis connection failed: {e}");
            return SessionEnd::ConnectionLost;
        }
    };
    let mut pubsub = conn.as_pubsub();
    for topic in topics {
        if let Err(e) = pubsub.subscribe(topic) {
            eprintln!("{name}: subscribe {topic} failed: {e}");
            return SessionEnd::ConnectionLost;
        }
    }
    eprintln!("{name}: subscribed to {} topic(s)", topics.len());
    loop {
        match pubsub.get_message() {
            Ok(msg) => {
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(_) => continue, // non-UTF8/wrong-type payload: skip
                };
                if on_payload(payload).is_break() {
                    return SessionEnd::ConsumerGone;
                }
            }
            Err(e) => {
                eprintln!("{name}: get_message error: {e}");
                return SessionEnd::Subscribed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let mut b = BACKOFF_MIN;
        let mut seen = vec![b];
        for _ in 0..6 {
            b = next_backoff(b);
            seen.push(b);
        }
        assert_eq!(seen[0], Duration::from_millis(250));
        assert_eq!(seen[1], Duration::from_millis(500));
        assert_eq!(seen[2], Duration::from_secs(1));
        assert_eq!(seen[3], Duration::from_secs(2));
        assert_eq!(seen[4], Duration::from_secs(4));
        assert_eq!(seen[5], Duration::from_secs(5), "capped");
        assert_eq!(seen[6], Duration::from_secs(5), "stays capped");
    }

    #[test]
    fn empty_topics_spawns_nothing() {
        // Must not spin a thread that subscribes to nothing (matches the old
        // neighbor-subscriber guard).
        spawn_resilient_subscriber("test", "redis://127.0.0.1:1".into(), vec![], |_| {
            panic!("no payload possible")
        });
    }
}
