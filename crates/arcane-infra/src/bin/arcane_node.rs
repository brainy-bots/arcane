//! Minimal node binary — infrastructure only, no demo/game logic.
//! Runs the node loop with client-driven state only. For demo agents (gravity, jump, etc.), use arcane-demo's arcane_node_demo binary.
//!
//! Env:
//!   NODE_ID                      — required; UUID of the cluster hosted by this node.
//!   REDIS_URL                    — optional; default `redis://127.0.0.1:6379`.
//!   NEIGHBOR_IDS                 — optional; comma-separated UUIDs of neighbor clusters.
//!   NODE_WS_PORT                 — optional (when built with --features cluster-ws); default 8080.
//!   NODE_CLIENT_IDLE_TIMEOUT_SECS — optional; client-driven entity idle timeout in seconds; default 0 (disabled).
//!
//! Features:
//!   When built with `--features migration`, enables the control-plane paths:
//!   - Inbox consumption at `arcane:inbox:<NODE_ID>` for ownership flip events.
//!   - State publication at `arcane:state:<NODE_ID>` every `NODE_STATE_PUBLISH_TICKS`.
//!   - First-sight ownership claiming for entities with no recorded owner.
//!
//! Example:
//!   NODE_ID=550e8400-e29b-41d4-a716-446655440000 cargo run -p arcane-infra --bin arcane-node --features cluster-ws
//!   NODE_ID=550e8400-e29b-41d4-a716-446655440000 cargo run -p arcane-infra --bin arcane-node --features cluster-ws,migration

use std::sync::Arc;

use arcane_core::ClusterSimulation;

#[cfg(feature = "cluster-ws")]
use arcane_infra::node_runner::{self, NodeEnv};

fn main() -> Result<(), String> {
    #[cfg(feature = "cluster-ws")]
    {
        arcane_infra::startup::raise_and_assert_fd_limit()?;
        let env = NodeEnv::from_env()?;
        node_runner::run_node_loop(
            env.cluster_id,
            env.redis_url,
            env.neighbor_ids,
            env.ws_port,
            |_| vec![],
            Option::<Arc<dyn ClusterSimulation>>::None,
        )
    }

    #[cfg(not(feature = "cluster-ws"))]
    {
        Err("cluster-ws feature required to run the node binary".to_string())
    }
}
