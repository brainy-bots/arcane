//! Cluster server run loop — library entry point for running a cluster with optional per-tick entity injection.
//! Used by the arcane-cluster binary (no demo) and by arcane-demo's cluster-demo binary (with demo agents).
//! Keeps infrastructure (this crate) free of game/demo logic.
//!
//! Interactions:
//! - pulls local simulation deltas from `ClusterServer`
//! - consumes neighbor deltas from `neighbor_subscriber`
//! - publishes merged state to `ws_server`
//! - optionally persists snapshots through `spacetimedb_persist`

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use arcane_core::cluster_simulation::{ClusterSimulation, GameAction};
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

fn merge_with_neighbor_latest(
    our_delta: EntityStateDelta,
    neighbor_latest: &HashMap<Uuid, Vec<EntityStateEntry>>,
) -> EntityStateDelta {
    let merged_updated: Vec<EntityStateEntry> = our_delta
        .updated
        .iter()
        .cloned()
        .chain(neighbor_latest.values().flat_map(|v| v.iter().cloned()))
        .collect();
    EntityStateDelta {
        source_cluster_id: our_delta.source_cluster_id,
        seq: our_delta.seq,
        tick: our_delta.tick,
        timestamp: our_delta.timestamp,
        updated: merged_updated,
        removed: our_delta.removed,
    }
}

/// Runs the cluster server loop with WebSocket and Redis replication.
/// Each tick, after applying client updates, calls `extra_entities_for_tick(tick_count)` and pushes any returned entries into the server (e.g. demo agents from arcane-demo).
///
/// When `simulation` is `Some`, [`ClusterSimulation::on_tick`] runs after those steps and before
/// [`ClusterServer::tick`], using `1 / TICK_RATE_HZ` as `dt_seconds`.
/// Never returns on success (infinite loop); returns Err only if setup fails.
#[cfg(feature = "cluster-ws")]
pub fn run_cluster_loop<F>(
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
    let replication = ReplicationChannelManager::new(cluster_id);
    replication
        .start(&redis_url)
        .map_err(|e| format!("Redis start failed: {}", e))?;
    replication.set_neighbors(neighbor_ids.clone());

    let server = ClusterServer::new(cluster_id);
    server.set_replication(Arc::new(replication));

    let (state_tx, state_rx) = std::sync::mpsc::channel();
    let (client_updates_tx, client_updates_rx) = std::sync::mpsc::channel();
    let (game_actions_tx, game_actions_rx) = std::sync::mpsc::channel::<GameAction>();
    crate::ws_server::run_ws_server(ws_port, state_rx, client_updates_tx, game_actions_tx);

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
    let dt_seconds = interval.as_secs_f64();
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
        let mut tick_actions: Vec<GameAction> = Vec::new();
        while let Ok(action) = game_actions_rx.try_recv() {
            tick_actions.push(action);
        }
        let tick_start = Instant::now();
        let upcoming_tick = server.current_tick() + 1;
        server.simulate_before_tick(
            dt_seconds,
            upcoming_tick,
            simulation.as_ref().map(|s| s.as_ref()),
            &tick_actions,
        );
        let our_delta = server.tick();
        let tick_elapsed_ms = tick_start.elapsed().as_secs_f64() * 1000.0;
        let merged_delta = merge_with_neighbor_latest(our_delta, &neighbor_latest);
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

#[cfg(test)]
mod tests {
    use super::merge_with_neighbor_latest;
    use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
    use arcane_core::Vec3;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn mk_entry(entity_id: Uuid, cluster_id: Uuid, x: f64) -> EntityStateEntry {
        EntityStateEntry::new(
            entity_id,
            cluster_id,
            Vec3::new(x, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
        )
    }

    #[test]
    fn merge_includes_local_and_latest_neighbor_entries() {
        let local_cluster = Uuid::from_u128(1);
        let n1 = Uuid::from_u128(2);
        let n2 = Uuid::from_u128(3);
        let local_entity = mk_entry(Uuid::from_u128(11), local_cluster, 10.0);
        let n1_entity = mk_entry(Uuid::from_u128(12), n1, 20.0);
        let n2_entity = mk_entry(Uuid::from_u128(13), n2, 30.0);

        let our_delta = EntityStateDelta {
            source_cluster_id: local_cluster,
            seq: 7,
            tick: 42,
            timestamp: 123.0,
            updated: vec![local_entity.clone()],
            removed: vec![Uuid::from_u128(99)],
        };
        let mut neighbors = HashMap::new();
        neighbors.insert(n1, vec![n1_entity.clone()]);
        neighbors.insert(n2, vec![n2_entity.clone()]);

        let merged = merge_with_neighbor_latest(our_delta, &neighbors);
        assert_eq!(merged.source_cluster_id, local_cluster);
        assert_eq!(merged.seq, 7);
        assert_eq!(merged.tick, 42);
        assert_eq!(merged.removed, vec![Uuid::from_u128(99)]);
        assert_eq!(merged.updated.len(), 3);
        assert!(merged
            .updated
            .iter()
            .any(|e| e.entity_id == local_entity.entity_id));
        assert!(merged
            .updated
            .iter()
            .any(|e| e.entity_id == n1_entity.entity_id));
        assert!(merged
            .updated
            .iter()
            .any(|e| e.entity_id == n2_entity.entity_id));
    }

    #[test]
    fn merge_uses_latest_neighbor_snapshot_for_each_cluster() {
        let local_cluster = Uuid::from_u128(1);
        let n1 = Uuid::from_u128(2);
        let old_n1_entity = mk_entry(Uuid::from_u128(21), n1, 1.0);
        let new_n1_entity = mk_entry(Uuid::from_u128(22), n1, 2.0);

        let mut neighbors = HashMap::new();
        neighbors.insert(n1, vec![old_n1_entity]);
        // Simulate loop behavior that replaces the last-seen snapshot for a neighbor.
        neighbors.insert(n1, vec![new_n1_entity.clone()]);

        let merged = merge_with_neighbor_latest(
            EntityStateDelta {
                source_cluster_id: local_cluster,
                seq: 1,
                tick: 1,
                timestamp: 0.0,
                updated: vec![],
                removed: vec![],
            },
            &neighbors,
        );
        assert_eq!(merged.updated.len(), 1);
        assert_eq!(merged.updated[0].entity_id, new_n1_entity.entity_id);
    }
}
