//! arcane-router — the data-plane worker (design §2.3).
//!
//! The execution half of the manager/router split: the MANAGER decides
//! (graph, prediction, partition) and writes per-cluster routing DOCS to the
//! routing table at decision cadence (~1 Hz). This binary is a stateless
//! ROUTER WORKER: at data cadence (default 10 Hz) it re-joins those docs with
//! FRESH entity state and publishes each cluster's inbox frame.
//!
//! Why the split matters: decisions age well (ownership and interest don't
//! flip in 100 ms) but state does not (a combat opponent's position from 1 s
//! ago is useless at 60 Hz sim). Each input is consumed at its natural rate,
//! and the attention spectrum's cadence gate gets a fast clock to express
//! rates against — a p≈1 opponent is delivered at the router rate, not the
//! decision rate.
//!
//! Scaling: workers are stateless and interchangeable (router scaling is
//! independent of cluster topology). `ROUTER_CLUSTERS` scopes this worker's
//! jobs; run several workers with disjoint scopes to spread load. Reads per
//! pass are bounded: one MGET for routing docs + one MGET for state docs.
//!
//! Env contract:
//!   ROUTER_CLUSTERS — REQUIRED: comma-separated cluster UUIDs this worker
//!     serves (its job scope).
//!   REDIS_URL — optional; default "redis://127.0.0.1:6379".
//!   ROUTER_TICK_MS — optional; default 100 (10 Hz).
//!
//! The manager keeps its own in-process routing pass unless
//! MANAGER_ROUTE=off — run this worker with that setting to take over
//! frame publication cleanly (both running is safe but doubles frames).

use std::env;
use std::time::{Duration, Instant};

use arcane_infra::node_inbox::{InboxBus, RedisInboxBus};
use arcane_infra::router_core::{route_from_doc, RouterConfig};

use arcane_infra::routing_table::{RedisRoutingTable, RoutingTable};
use arcane_infra::state_keys::RedisStateSource;
use uuid::Uuid;

fn main() -> Result<(), String> {
    let clusters: Vec<Uuid> = env::var("ROUTER_CLUSTERS")
        .map_err(|_| "ROUTER_CLUSTERS env var required (comma-separated UUIDs)".to_string())?
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| Uuid::parse_str(s).map_err(|e| format!("invalid cluster id {s}: {e}")))
        .collect::<Result<_, _>>()?;
    if clusters.is_empty() {
        return Err("ROUTER_CLUSTERS parsed empty".into());
    }
    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let tick_ms: u64 = env::var("ROUTER_TICK_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let mut table = RedisRoutingTable::new(&redis_url)?;
    let state_source = RedisStateSource::new(&redis_url, clusters.clone())?;
    let bus = RedisInboxBus::new(&redis_url)?;
    let config = RouterConfig {
        router_hz: 1000.0 / tick_ms as f64,
        ..Default::default()
    };

    eprintln!(
        "arcane-router: {} cluster jobs at {} Hz (ROUTER_TICK_MS={tick_ms})",
        clusters.len(),
        config.router_hz
    );

    let interval = Duration::from_millis(tick_ms);
    let mut router_tick: u64 = 0;
    let mut last_log = Instant::now();
    let mut frames_published: u64 = 0;

    loop {
        let pass_start = Instant::now();
        router_tick += 1;

        // Read 1: this worker's routing docs (one MGET).
        let docs = match table.read(&clusters) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("arcane-router: table read failed ({e}); retrying");
                std::thread::sleep(interval);
                continue;
            }
        };
        if docs.is_empty() {
            // Manager hasn't written yet — idle politely.
            std::thread::sleep(interval);
            continue;
        }

        // Read 2: fresh entity state (one MGET across the state keys; the
        // docs' interest owners are all clusters in this deployment, so the
        // full fetch IS the bounded join).
        let records = state_source.fetch_all();
        let entity_states = arcane_infra::router_core::entity_states_from_records(&records);

        // Route + publish: fresh state joined with the last decisions.
        for (cluster, doc) in &docs {
            let frame = route_from_doc(doc, &entity_states, &config, router_tick);
            if bus.publish(*cluster, frame).is_err() {
                eprintln!("arcane-router: publish to {cluster} failed");
            } else {
                frames_published += 1;
            }
        }

        if last_log.elapsed() > Duration::from_secs(10) {
            eprintln!(
                "arcane-router: tick {router_tick}, {frames_published} frames published, \
                 {} entities in state view",
                entity_states.len()
            );
            last_log = Instant::now();
        }

        let elapsed = pass_start.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }
    }
}
