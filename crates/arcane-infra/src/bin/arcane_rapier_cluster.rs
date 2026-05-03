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

use std::env;
use std::sync::Arc;

use arcane_core::ClusterSimulation;
use arcane_infra::{cluster_runner, RapierClusterSim, RapierConfig};
use uuid::Uuid;

fn parse_uuids(s: &str) -> Vec<Uuid> {
    s.split(',')
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .filter_map(|x| Uuid::parse_str(x).ok())
        .collect()
}

fn main() -> Result<(), String> {
    let cluster_id =
        env::var("CLUSTER_ID").map_err(|_| "CLUSTER_ID env var required (UUID)".to_string())?;
    let cluster_id =
        Uuid::parse_str(&cluster_id).map_err(|e| format!("invalid CLUSTER_ID: {}", e))?;

    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let neighbor_ids = env::var("NEIGHBOR_IDS")
        .map(|s| parse_uuids(&s))
        .unwrap_or_default();

    let ws_port: u16 = env::var("CLUSTER_WS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let user_sim: Option<Arc<dyn ClusterSimulation>> = None;
    let rapier_sim: Arc<dyn ClusterSimulation> =
        Arc::new(RapierClusterSim::new(user_sim, RapierConfig::default()));

    cluster_runner::run_cluster_loop(
        cluster_id,
        redis_url,
        neighbor_ids,
        ws_port,
        |_| vec![],
        Some(rapier_sim),
    )
}
