//! Rapier-backed cluster server binary.
//!
//! Same env vars and command shape as `arcane_cluster.rs`. The only difference:
//! the user simulation is wrapped in [`arcane_infra::rapier_cluster::RapierClusterSim`],
//! so authoritative pose advancement happens through Rapier instead of the user's
//! `on_tick`. Networking, replication, neighbor merge, and persistence are
//! identical to the vanilla cluster.
//!
//! Env (same as arcane-cluster):
//!   CLUSTER_ID      — required; UUID of this cluster.
//!   REDIS_URL       — optional; default `redis://127.0.0.1:6379`.
//!   NEIGHBOR_IDS    — optional; comma-separated UUIDs of neighbor clusters.
//!   CLUSTER_WS_PORT — optional; default 8080.

use std::sync::Arc;

use arcane_core::ClusterSimulation;
use arcane_infra::cluster_runner::{self, ClusterEnv};
use arcane_infra::{RapierClusterSim, RapierConfig};

fn main() -> Result<(), String> {
    let env = ClusterEnv::from_env()?;

    let user_sim: Option<Arc<dyn ClusterSimulation>> = None;
    let rapier_sim: Arc<dyn ClusterSimulation> =
        Arc::new(RapierClusterSim::new(user_sim, RapierConfig::default()));

    cluster_runner::run_cluster_loop(
        env.cluster_id,
        env.redis_url,
        env.neighbor_ids,
        env.ws_port,
        |_| vec![],
        Some(rapier_sim),
    )
}
