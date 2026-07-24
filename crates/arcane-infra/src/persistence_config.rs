//! Persistence layer configuration and construction (L2, epic #305).
//!
//! Resolves the persistence level from environment variables and constructs
//! the appropriate implementation. Supports three levels:
//! - L0 (none): no persistence
//! - L1 (short): Redis TTL-based snapshots (via parking module)
//! - L2 (full): durable backend (SpacetimeDB or custom via IPersistence)

use arcane_core::IPersistence;
use std::sync::Arc;

#[cfg(feature = "spacetimedb-persist")]
use crate::spacetimedb_persist::SpacetimeDbPersist;

/// Construct a persistence implementation from environment variables.
///
/// **Environment variables:**
/// - `ARCANE_PERSISTENCE`: level control — `none`, `short`, or `full` (default: `short`)
/// - `SPACETIMEDB_PERSIST=1`: legacy back-compat mapping to `full` (logs deprecation warning)
///
/// Returns:
/// - `None` if persistence is disabled (L0 or unavailable)
/// - `Some(Arc<dyn IPersistence>)` for L2 when enabled (SpacetimeDB if `spacetimedb-persist` feature enabled, else None with log)
pub fn construct_persistence() -> Option<Arc<dyn IPersistence>> {
    let persistence_level = resolve_persistence_level();

    match persistence_level.as_str() {
        "none" => {
            eprintln!("[persistence] L0: no persistence configured");
            None
        }
        "short" => {
            eprintln!("[persistence] L1: short-term (Redis parking); L2 durable disabled");
            None
        }
        "full" => {
            #[cfg(feature = "spacetimedb-persist")]
            {
                if let Some(persist) = SpacetimeDbPersist::from_env() {
                    eprintln!("[persistence] L2: SpacetimeDB durable backend (full)");
                    return Some(Arc::new(persist));
                } else {
                    eprintln!("[persistence] L2 (full) requested but SpacetimeDB not configured; running without durable persistence");
                    return None;
                }
            }

            #[cfg(not(feature = "spacetimedb-persist"))]
            {
                eprintln!("[persistence] L2 (full) requested but spacetimedb-persist feature not enabled; running without durable persistence");
                None
            }
        }
        other => {
            eprintln!(
                "[persistence] unknown level '{}'; defaulting to L1 (short)",
                other
            );
            None
        }
    }
}

/// Resolve the persistence level from environment variables.
/// Default is `short` (L1 parking only).
fn resolve_persistence_level() -> String {
    // Check for new ARCANE_PERSISTENCE variable first
    if let Ok(level) = std::env::var("ARCANE_PERSISTENCE") {
        return level.to_lowercase();
    }

    // Back-compat: SPACETIMEDB_PERSIST=1 maps to full, with deprecation warning
    if let Ok(val) = std::env::var("SPACETIMEDB_PERSIST") {
        if val == "1" || val.eq_ignore_ascii_case("true") {
            eprintln!("[persistence] SPACETIMEDB_PERSIST=1 is deprecated; use ARCANE_PERSISTENCE=full instead");
            return "full".to_string();
        }
    }

    // Default: L1 short-term persistence (Redis parking via parking module)
    "short".to_string()
}
