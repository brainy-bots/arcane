//! Cluster server run loop — library entry point for running a cluster with optional per-tick entity injection.
//! Used by the arcane-cluster binary (no demo) and by arcane-demo's cluster-demo binary (with demo agents).
//! Keeps infrastructure (this crate) free of game/demo logic.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
use uuid::Uuid;

#[cfg(feature = "cluster-ws")]
use crate::neighbor_subscriber::spawn_neighbor_subscriber;
#[cfg(feature = "spacetimedb-persist")]
use crate::spacetimedb_persist::SpacetimeDbPersist;
use crate::{ClusterServer, ReplicationChannelManager};

const TICK_RATE_HZ: u64 = 20;
const LOG_EVERY_TICKS: u64 = 100;
/// Log parseable server stats every N ticks (for benchmark: entities, clusters, tick_ms).
const LOG_STATS_EVERY_TICKS: u64 = 40;

/// Runs the cluster server loop with WebSocket and Redis replication.
/// Each tick, after applying client updates, calls `extra_entities_for_tick(tick_count)` and pushes any returned entries into the server (e.g. demo agents from arcane-demo).
/// Never returns on success (infinite loop); returns Err only if setup fails.
#[cfg(feature = "cluster-ws")]
pub fn run_cluster_loop<F>(
    cluster_id: Uuid,
    redis_url: String,
    neighbor_ids: Vec<Uuid>,
    ws_port: u16,
    mut extra_entities_for_tick: F,
) -> Result<(), String>
where
    F: FnMut(u64) -> Vec<EntityStateEntry>,
{
    let replication = ReplicationChannelManager::new(cluster_id);
    replication
        .start(&redis_url)
        .map_err(|e| format!("Redis start failed: {}", e))?;
    replication.set_neighbors(neighbor_ids.clone());

    let server = ClusterServer::new(cluster_id);
    server.set_replication(Arc::new(replication));

    let (state_tx, state_rx) = std::sync::mpsc::channel();
    let (client_updates_tx, client_updates_rx) = std::sync::mpsc::channel();
    crate::ws_server::run_ws_server(ws_port, state_rx, client_updates_tx);

    let (neighbor_tx, neighbor_rx) = std::sync::mpsc::channel::<EntityStateDelta>();
    spawn_neighbor_subscriber(redis_url.clone(), neighbor_ids.clone(), neighbor_tx);
    let mut neighbor_latest: HashMap<Uuid, Vec<EntityStateEntry>> = HashMap::new();

    eprintln!(
        "arcane-cluster started cluster_id={} neighbors={} tick_rate={}Hz",
        cluster_id,
        neighbor_ids.len(),
        TICK_RATE_HZ
    );

    #[cfg(feature = "spacetimedb-persist")]
    let persist = SpacetimeDbPersist::from_env();

    let interval = Duration::from_millis(1000 / TICK_RATE_HZ);
    let mut tick_count: u64 = 0;

    loop {
        while let Ok(mut entry) = client_updates_rx.try_recv() {
            entry.cluster_id = cluster_id;
            server.add_entity(entry);
        }
        for mut entry in extra_entities_for_tick(tick_count) {
            entry.cluster_id = cluster_id;
            server.add_entity(entry);
        }
        while let Ok(delta) = neighbor_rx.try_recv() {
            neighbor_latest.insert(delta.source_cluster_id, delta.updated);
        }
        let tick_start = Instant::now();
        let our_delta = server.tick();
        let tick_elapsed_ms = tick_start.elapsed().as_secs_f64() * 1000.0;
        let merged_updated: Vec<EntityStateEntry> = our_delta
            .updated
            .iter()
            .cloned()
            .chain(neighbor_latest.values().flat_map(|v| v.iter().cloned()))
            .collect();
        let merged_delta = EntityStateDelta {
            source_cluster_id: our_delta.source_cluster_id,
            seq: our_delta.seq,
            tick: our_delta.tick,
            timestamp: our_delta.timestamp,
            updated: merged_updated,
            removed: our_delta.removed,
        };
        #[cfg(feature = "spacetimedb-persist")]
        if let Some(ref persist) = persist {
            persist.maybe_persist(tick_count, &merged_delta.updated);
        }

        let _ = state_tx.send(merged_delta);

        tick_count += 1;
        if tick_count.is_multiple_of(LOG_EVERY_TICKS) {
            eprintln!(
                "tick {} seq {}",
                server.current_tick(),
                server.current_seq()
            );
        }
        if tick_count.is_multiple_of(LOG_STATS_EVERY_TICKS) {
            let entities = server.entity_count();
            let clusters = 1u32; // This process is one cluster; multi-cluster = multiple processes
            eprintln!(
                "ArcaneServerStats: entities={} clusters={} tick_ms={:.2}",
                entities, clusters, tick_elapsed_ms
            );
        }
        thread::sleep(interval);
    }
    // unreachable
}
