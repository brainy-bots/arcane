//! Rapier-backed node binary.
//!
//! Same env vars and command shape as `arcane_node.rs`. The only difference:
//! the user simulation is wrapped in [`arcane_infra::rapier_cluster::RapierClusterSim`],
//! so authoritative pose advancement happens through Rapier instead of the user's
//! `on_tick`. Networking, replication, neighbor merge, and persistence are
//! identical to the vanilla node.
//!
//! Env (same as arcane-node):
//!   NODE_ID      — required; UUID of the cluster hosted by this node.
//!   REDIS_URL    — optional; default `redis://127.0.0.1:6379`.
//!   NEIGHBOR_IDS — optional; comma-separated UUIDs of neighbor clusters.
//!   NODE_WS_PORT — optional; default 8080.

use std::sync::Arc;

use arcane_core::ClusterSimulation;
use arcane_infra::node_runner::{self, NodeEnv};
use arcane_infra::{RapierClusterSim, RapierConfig};

fn main() -> Result<(), String> {
    arcane_infra::startup::raise_and_assert_fd_limit()?;
    let env = NodeEnv::from_env()?;

    let user_sim: Option<Arc<dyn ClusterSimulation>> = None;
    let rapier_sim: Arc<dyn ClusterSimulation> =
        Arc::new(RapierClusterSim::new(user_sim, RapierConfig::default()));

    node_runner::run_node_loop(
        env.cluster_id,
        env.redis_url,
        env.neighbor_ids,
        env.ws_port,
        |_| vec![],
        Some(rapier_sim),
    )
}
