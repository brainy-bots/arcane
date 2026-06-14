//! Node run loop — library entry point for running a node with optional per-tick entity injection.
//! Used by the arcane-node binary (no demo) and by arcane-demo's node-demo binary (with demo agents).
//! Keeps infrastructure (this crate) free of game/demo logic.
//!
//! Interactions:
//! - pulls local simulation deltas from `ArcaneNode`
//! - consumes neighbor deltas from `neighbor_subscriber`
//! - publishes merged state to `ws_server`
//! - optionally persists snapshots through `spacetimedb_persist`

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use arcane_core::cluster_simulation::ClusterSimulation;
use arcane_core::replication_channel::EntityStateEntry;
use uuid::Uuid;

#[cfg(feature = "cluster-ws")]
use crate::node_core::{NodeConfig, NodeCore};

/// Node-binary environment configuration (NODE_ID, REDIS_URL,
/// NEIGHBOR_IDS, NODE_WS_PORT). Shared by every node-binary entry point
/// so the env contract stays in one place.
#[derive(Clone, Debug)]
pub struct NodeEnv {
    pub cluster_id: Uuid,
    pub redis_url: String,
    pub neighbor_ids: Vec<Uuid>,
    pub ws_port: u16,
}

impl NodeEnv {
    /// Read the standard node env vars. `NODE_ID` is required; the rest
    /// have defaults (Redis at `127.0.0.1:6379`, no neighbors, WS port `8080`).
    pub fn from_env() -> Result<Self, String> {
        let cluster_id =
            std::env::var("NODE_ID").map_err(|_| "NODE_ID env var required (UUID)".to_string())?;
        let cluster_id =
            Uuid::parse_str(&cluster_id).map_err(|e| format!("invalid NODE_ID: {}", e))?;
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let neighbor_ids = std::env::var("NEIGHBOR_IDS")
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim())
                    .filter(|x| !x.is_empty())
                    .filter_map(|x| Uuid::parse_str(x).ok())
                    .collect()
            })
            .unwrap_or_default();
        let ws_port: u16 = std::env::var("NODE_WS_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8080);
        Ok(Self {
            cluster_id,
            redis_url,
            neighbor_ids,
            ws_port,
        })
    }
}

/// Runs the node server loop with WebSocket and Redis replication.
/// Each tick, after applying client updates, calls `extra_entities_for_tick(tick_count)` and pushes any returned entries into the server (e.g. demo agents from arcane-demo).
///
/// When `simulation` is `Some`, [`ClusterSimulation::on_tick`] runs after those steps and before
/// [`ArcaneNode::tick`], using `1 / tick_rate_hz()` as `dt_seconds` (env-driven, see
/// [`crate::tick_rate`]). Never returns on success (infinite loop); returns Err only if setup fails.
#[cfg(feature = "cluster-ws")]
pub fn run_node_loop<F>(
    cluster_id: Uuid,
    redis_url: String,
    neighbor_ids: Vec<Uuid>,
    ws_port: u16,
    mut extra_entities_for_tick: F,
    simulation: Option<Arc<dyn ClusterSimulation>>,
) -> Result<(), String>
where
    F: FnMut(u64) -> Vec<EntityStateEntry>,
{
    let mut core = NodeCore::new(NodeConfig {
        cluster_id,
        redis_url,
        neighbor_ids,
        ws_port,
    })?;

    let tick_rate_hz = crate::tick_rate::tick_rate_hz();
    let interval = Duration::from_millis(1000 / tick_rate_hz);

    loop {
        let extra = extra_entities_for_tick(core.tick_count());
        core.tick(simulation.as_ref().map(|s| s.as_ref()), extra);
        thread::sleep(interval);
    }
    // unreachable
}
