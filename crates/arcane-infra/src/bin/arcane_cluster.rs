//! Minimal cluster server binary — infrastructure only, no demo/game logic.
//! Runs the cluster loop with client-driven state only. For demo agents (gravity, jump, etc.), use arcane-demo's arcane_cluster_demo binary.
//!
//! Env:
//!   CLUSTER_ID      — required; UUID of this cluster.
//!   REDIS_URL       — optional; default `redis://127.0.0.1:6379`.
//!   NEIGHBOR_IDS    — optional; comma-separated UUIDs of neighbor clusters.
//!   CLUSTER_WS_PORT — optional (when built with --features cluster-ws); default 8080.
//!
//! Example:
//!   CLUSTER_ID=550e8400-e29b-41d4-a716-446655440000 cargo run -p arcane-infra --bin arcane-cluster --features cluster-ws

use std::sync::Arc;

use arcane_core::ClusterSimulation;

#[cfg(feature = "cluster-ws")]
use arcane_infra::cluster_runner::{self, ClusterEnv};

fn main() -> Result<(), String> {
    #[cfg(feature = "cluster-ws")]
    {
        arcane_infra::startup::raise_and_assert_fd_limit()?;
        let env = ClusterEnv::from_env()?;
        cluster_runner::run_cluster_loop(
            env.cluster_id,
            env.redis_url,
            env.neighbor_ids,
            env.ws_port,
            |_| vec![], // no demo entities; use arcane_cluster_demo from arcane-demo for that
            Option::<Arc<dyn ClusterSimulation>>::None,
        )
    }

    #[cfg(not(feature = "cluster-ws"))]
    {
        Err("cluster-ws feature required to run the cluster binary".to_string())
    }
}
