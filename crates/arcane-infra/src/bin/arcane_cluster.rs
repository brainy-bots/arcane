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

use std::env;
use std::sync::Arc;

use arcane_core::ClusterSimulation;
use arcane_infra::cluster_runner;
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

    #[cfg(feature = "cluster-ws")]
    {
        cluster_runner::run_cluster_loop(
            cluster_id,
            redis_url,
            neighbor_ids,
            ws_port,
            |_| vec![], // no demo entities; use arcane_cluster_demo from arcane-demo for that
            Option::<Arc<dyn ClusterSimulation>>::None,
        )
    }

    #[cfg(not(feature = "cluster-ws"))]
    {
        let _ = (cluster_id, redis_url, neighbor_ids, ws_port);
        Err("cluster-ws feature required to run the cluster binary".to_string())
    }
}
