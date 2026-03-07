//! Manager HTTP service. Clients hit /join to get cluster assignment (cluster_id, server_host, server_port).
//! Supports a single cluster (MANAGER_CLUSTER_ID + MANAGER_SERVER_*) or multiple clusters (MANAGER_CLUSTERS) with round-robin.
//!
//! Env (single cluster, backward compatible):
//!   MANAGER_CLUSTER_ID   — UUID of the cluster.
//!   MANAGER_SERVER_HOST  — optional; default `127.0.0.1`.
//!   MANAGER_SERVER_PORT  — optional; default `8080`.
//!
//! Env (multiple clusters):
//!   MANAGER_CLUSTERS     — comma-separated entries, each "cluster_id:host:port" (e.g. "uuid1:127.0.0.1:8080,uuid2:127.0.0.1:8082").
//!
//! Env (both):
//!   MANAGER_HTTP_PORT    — optional; default `8081`.

use std::env;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use serde::Serialize;

#[derive(Clone, Serialize)]
struct JoinResponse {
    cluster_id: String,
    server_host: String,
    server_port: u16,
}

#[derive(Clone)]
struct ManagerState {
    clusters: Vec<(String, String, u16)>,
    counter: std::sync::Arc<AtomicUsize>,
}

fn parse_clusters(s: &str) -> Vec<(String, String, u16)> {
    s.split(',')
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .filter_map(|e| {
            let parts: Vec<&str> = e.splitn(3, ':').collect();
            if parts.len() != 3 {
                return None;
            }
            let port: u16 = parts[2].parse().ok()?;
            Some((parts[0].to_string(), parts[1].to_string(), port))
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let http_port: u16 = env::var("MANAGER_HTTP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8081);

    let state = if let Ok(clusters_var) = env::var("MANAGER_CLUSTERS") {
        let clusters = parse_clusters(&clusters_var);
        if clusters.is_empty() {
            return Err("MANAGER_CLUSTERS is set but parsed to empty (use id:host:port,id2:host2:port2)".to_string());
        }
        eprintln!("arcane-manager: {} cluster(s), round-robin assign", clusters.len());
        ManagerState {
            clusters: clusters.clone(),
            counter: std::sync::Arc::new(AtomicUsize::new(0)),
        }
    } else {
        let cluster_id = env::var("MANAGER_CLUSTER_ID")
            .map_err(|_| "MANAGER_CLUSTER_ID or MANAGER_CLUSTERS env var required".to_string())?;
        let server_host = env::var("MANAGER_SERVER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let server_port: u16 = env::var("MANAGER_SERVER_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8080);
        let clusters = vec![(cluster_id, server_host, server_port)];
        ManagerState {
            clusters: clusters.clone(),
            counter: std::sync::Arc::new(AtomicUsize::new(0)),
        }
    };

    let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    eprintln!("arcane-manager listening on http://{}", addr);

    let app = Router::new()
        .route("/join", get(join_handler))
        .with_state(state);

    axum::serve(
        tokio::net::TcpListener::bind(addr).await.map_err(|e| e.to_string())?,
        app,
    )
    .await
    .map_err(|e| e.to_string())
}

async fn join_handler(State(s): State<ManagerState>) -> Json<JoinResponse> {
    let idx = s.counter.fetch_add(1, Ordering::Relaxed) % s.clusters.len();
    let (cluster_id, server_host, server_port) = &s.clusters[idx];
    Json(JoinResponse {
        cluster_id: cluster_id.clone(),
        server_host: server_host.clone(),
        server_port: *server_port,
    })
}
