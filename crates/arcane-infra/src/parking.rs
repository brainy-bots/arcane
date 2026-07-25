//! Short-term persistence layer: park entity snapshots in Redis with TTL for reconnection.
//!
//! Epic #305, sub-issue #307: L1 short-term persistence. When a player leaves,
//! their entity snapshot (spine + user_data, NOT local_data) is serialized and stored
//! in Redis with a TTL. If they rejoin within the TTL, the snapshot is restored.
//!
//! **Env config:**
//! - `ARCANE_RECONNECT_TTL_SECS`: How long to keep parked snapshots (default 120).
//!   Set to 0 to disable parking entirely (pure L0).
//!
//! **Redis keys:**
//! - `arcane:parked:{entity_id}` — parked entity snapshot, expires after TTL.

use arcane_core::replication_channel::EntityStateEntry;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Process-wide cache of persistent Redis connections, one per URL.
///
/// Parking I/O used to open a NEW TCP connection per call. At the designed
/// rate (one park per player leave) that was tolerable; combined with a hot
/// loop it exhausted the OS ephemeral port table (Windows error 10048: every
/// port in TIME_WAIT), which then broke UNRELATED connects — including the
/// WS reconnects that complete migrations. Live-observed as “entities stuck
/// mid-migration”. A cached connection makes parking I/O O(process), not
/// O(calls); on any Redis error the connection is dropped and rebuilt next
/// call (Redis auto-reconnect semantics).
static CONNECTIONS: Mutex<Option<HashMap<String, redis::Connection>>> = Mutex::new(None);

/// Run `f` on the cached connection for `redis_url`, creating it if needed.
/// On error the cached connection is discarded so the next call reconnects.
fn with_connection<T>(
    redis_url: &str,
    f: impl FnOnce(&mut redis::Connection) -> redis::RedisResult<T>,
) -> redis::RedisResult<T> {
    let mut guard = CONNECTIONS.lock().unwrap_or_else(|p| p.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if !map.contains_key(redis_url) {
        let client = redis::Client::open(redis_url)?;
        let conn = client.get_connection()?;
        map.insert(redis_url.to_string(), conn);
    }
    let conn = map.get_mut(redis_url).expect("just inserted");
    let result = f(conn);
    if result.is_err() {
        map.remove(redis_url); // poisoned/broken: rebuild on next call
    }
    result
}

/// Configuration for parking behavior.
#[derive(Clone, Debug)]
pub struct ParkingConfig {
    /// TTL in seconds for parked entities. 0 disables parking.
    pub reconnect_ttl_secs: u64,
}

impl ParkingConfig {
    /// Load config from environment variables.
    pub fn from_env() -> Self {
        let reconnect_ttl_secs = std::env::var("ARCANE_RECONNECT_TTL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(120);

        Self { reconnect_ttl_secs }
    }

    /// Check if parking is enabled.
    pub fn is_enabled(&self) -> bool {
        self.reconnect_ttl_secs > 0
    }
}

/// Redis key for a parked entity snapshot.
fn parked_key(entity_id: Uuid) -> String {
    format!("arcane:parked:{}", entity_id.hyphenated())
}

/// Park an entity snapshot in Redis with TTL.
/// Serializes spine + user_data (NOT local_data); logs failures loudly.
pub fn park_entity(
    redis_url: &str,
    config: &ParkingConfig,
    entity_id: Uuid,
    entry: &EntityStateEntry,
) {
    if !config.is_enabled() {
        return;
    }

    // Serialize: spine (id, cluster_id, position, velocity) + user_data.
    // Create a minimal snapshot (don't serialize local_data).
    let snapshot = serde_json::json!({
        "entity_id": entry.entity_id,
        "cluster_id": entry.cluster_id,
        "position": {
            "x": entry.position.x,
            "y": entry.position.y,
            "z": entry.position.z,
        },
        "velocity": {
            "x": entry.velocity.x,
            "y": entry.velocity.y,
            "z": entry.velocity.z,
        },
        "user_data": entry.user_data,
    });

    let json_str = match serde_json::to_string(&snapshot) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "parking: JSON serialization failed for {}: {}",
                entity_id, e
            );
            return;
        }
    };

    let key = parked_key(entity_id);
    let ttl_secs = config.reconnect_ttl_secs;

    // SET key value EX ttl_secs (pooled connection; see CONNECTIONS).
    let res: Result<(), redis::RedisError> = with_connection(redis_url, |conn| {
        redis::cmd("SET")
            .arg(&key)
            .arg(&json_str)
            .arg("EX")
            .arg(ttl_secs)
            .query(conn)
    });

    match res {
        Ok(()) => {
            eprintln!(
                "[park] entity {} parked in Redis with TTL {}s",
                entity_id, ttl_secs
            );
        }
        Err(e) => {
            eprintln!("parking: Redis SET failed for {}: {}", entity_id, e);
        }
    }
}

/// Check if an entity is parked in Redis without consuming it.
pub fn is_entity_parked(redis_url: &str, entity_id: Uuid) -> bool {
    let key = parked_key(entity_id);
    with_connection(redis_url, |conn| {
        redis::cmd("EXISTS").arg(&key).query::<i32>(conn)
    })
    .map(|exists| exists > 0)
    .unwrap_or(false)
}

/// Retrieve and consume a parked entity snapshot from Redis.
/// Returns the snapshot JSON if found and deleted; None if expired or missing.
pub fn unpark_entity(redis_url: &str, entity_id: Uuid) -> Option<serde_json::Value> {
    let key = parked_key(entity_id);

    // GETDEL: atomic consume-once (replaces the old GET-then-DEL pair — one
    // round trip, no race window). Pooled connection; errors log once and
    // return None (caller treats it as “no snapshot”).
    let json_str: Option<String> =
        match with_connection(redis_url, |conn| redis::cmd("GETDEL").arg(&key).query(conn)) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("unpark: Redis error for {}: {}", entity_id, e);
                return None;
            }
        };

    let json_str = json_str?;

    // Deserialize the snapshot.
    match serde_json::from_str::<serde_json::Value>(&json_str) {
        Ok(snapshot) => {
            eprintln!(
                "[unpark] entity {} restored from parked snapshot",
                entity_id
            );
            Some(snapshot)
        }
        Err(e) => {
            eprintln!(
                "unpark: JSON deserialization failed for {}: {}",
                entity_id, e
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parked_key_format() {
        let id = Uuid::nil();
        let key = parked_key(id);
        assert_eq!(key, "arcane:parked:00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn test_parking_config_from_env() {
        // Test default when not set
        let config = ParkingConfig {
            reconnect_ttl_secs: 120,
        };
        assert!(config.is_enabled());
    }

    #[test]
    fn test_parking_disabled() {
        let config = ParkingConfig {
            reconnect_ttl_secs: 0,
        };
        assert!(!config.is_enabled());
    }
}
