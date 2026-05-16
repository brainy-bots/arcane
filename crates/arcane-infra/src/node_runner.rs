//! Node run loop — library entry point for running a node with optional per-tick entity injection.
//! Used by the arcane-node binary (no demo) and by arcane-demo's node-demo binary (with demo agents).
//! Keeps infrastructure (this crate) free of game/demo logic.
//!
//! Interactions:
//! - pulls local simulation deltas from `ArcaneNode`
//! - consumes neighbor deltas from `neighbor_subscriber`
//! - publishes merged state to `ws_server`
//! - optionally persists snapshots through `spacetimedb_persist`

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use std::sync::atomic::Ordering;

use arcane_core::cluster_simulation::{ClusterSimulation, GameAction};
use arcane_core::physics_events::PhysicsEventBatch;
use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
use uuid::Uuid;

#[cfg(feature = "cluster-ws")]
use crate::neighbor_subscriber::spawn_neighbor_subscriber;
#[cfg(feature = "cluster-ws")]
use crate::node_stats::{serve_stats_http, NodeStats};
#[cfg(feature = "cluster-ws")]
use crate::physics_events_channel::{spawn_physics_events_subscriber, PhysicsEventsPublisher};
#[cfg(feature = "spacetimedb-persist")]
use crate::spacetimedb_persist::SpacetimeDbPersist;
use crate::{ArcaneNode, ReplicationChannelManager};

const LOG_EVERY_TICKS: u64 = 100;
/// Log parseable server stats every N ticks (for benchmark: entities, clusters, tick_ms).
const LOG_STATS_EVERY_TICKS: u64 = 40;
/// Entities not updated by a neighbor for this many ticks are pruned (stale neighbor crash guard).
const NEIGHBOR_STALE_TICKS: u64 = 300;

/// Node-binary environment configuration (NODE_ID, REDIS_URL,
/// NEIGHBOR_IDS, NODE_WS_PORT). Shared by every node-binary entry point
/// so the env contract stays in one place.
#[derive(Clone, Debug)]
pub struct NodeEnv {
    pub cluster_id: Uuid,
    pub redis_url: String,
    pub neighbor_ids: Vec<Uuid>,
    pub ws_port: u16,
}

impl NodeEnv {
    /// Read the standard node env vars. `NODE_ID` is required; the rest
    /// have defaults (Redis at `127.0.0.1:6379`, no neighbors, WS port `8080`).
    pub fn from_env() -> Result<Self, String> {
        let cluster_id =
            std::env::var("NODE_ID").map_err(|_| "NODE_ID env var required (UUID)".to_string())?;
        let cluster_id =
            Uuid::parse_str(&cluster_id).map_err(|e| format!("invalid NODE_ID: {}", e))?;
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let neighbor_ids = std::env::var("NEIGHBOR_IDS")
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim())
                    .filter(|x| !x.is_empty())
                    .filter_map(|x| Uuid::parse_str(x).ok())
                    .collect()
            })
            .unwrap_or_default();
        let ws_port: u16 = std::env::var("NODE_WS_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8080);
        Ok(Self {
            cluster_id,
            redis_url,
            neighbor_ids,
            ws_port,
        })
    }
}

fn merge_with_neighbor_latest(
    our_delta: EntityStateDelta,
    neighbor_entities: &HashMap<Uuid, EntityStateEntry>,
) -> EntityStateDelta {
    let local_ids: HashSet<Uuid> = our_delta.updated.iter().map(|e| e.entity_id).collect();
    let merged_updated: Vec<EntityStateEntry> = our_delta
        .updated
        .into_iter()
        .chain(
            neighbor_entities
                .values()
                .filter(|e| !local_ids.contains(&e.entity_id))
                .cloned(),
        )
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

/// Runs the node server loop with WebSocket and Redis replication.
/// Each tick, after applying client updates, calls `extra_entities_for_tick(tick_count)` and pushes any returned entries into the server (e.g. demo agents from arcane-demo).
///
/// When `simulation` is `Some`, [`ClusterSimulation::on_tick`] runs after those steps and before
/// [`ArcaneNode::tick`], using `1 / tick_rate_hz()` as `dt_seconds` (env-driven, see
/// [`crate::tick_rate`]). Never returns on success (infinite loop); returns Err only if setup fails.
#[cfg(feature = "cluster-ws")]
pub fn run_node_loop<F>(
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

    let server = ArcaneNode::new(cluster_id);
    server.set_replication(Arc::new(replication));

    let (state_tx, state_rx) = std::sync::mpsc::channel();
    let (client_updates_tx, client_updates_rx) = std::sync::mpsc::channel();
    let (game_actions_tx, game_actions_rx) = std::sync::mpsc::channel::<GameAction>();

    let stats = NodeStats::new();
    let stats_port = std::env::var("NODE_STATS_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(ws_port.saturating_add(1));
    serve_stats_http(stats_port, cluster_id.to_string(), stats.clone());

    crate::ws_server::run_ws_server(
        ws_port,
        state_rx,
        client_updates_tx,
        game_actions_tx,
        stats.clone(),
    );

    let (neighbor_tx, neighbor_rx) = std::sync::mpsc::channel::<EntityStateDelta>();
    spawn_neighbor_subscriber(redis_url.clone(), neighbor_ids.clone(), neighbor_tx);
    let mut neighbor_entities: HashMap<Uuid, EntityStateEntry> = HashMap::new();
    let mut neighbor_last_seen: HashMap<Uuid, u64> = HashMap::new();

    let (physics_events_tx, physics_events_rx) = std::sync::mpsc::channel::<PhysicsEventBatch>();
    spawn_physics_events_subscriber(redis_url.clone(), cluster_id, physics_events_tx);
    let physics_publisher = PhysicsEventsPublisher::new(&redis_url)
        .map_err(|e| format!("physics events publisher: {}", e))?;

    let tick_rate_hz = crate::tick_rate::tick_rate_hz();
    eprintln!(
        "arcane-node started cluster_id={} neighbors={} tick_rate={}Hz",
        cluster_id,
        neighbor_ids.len(),
        tick_rate_hz
    );

    #[cfg(feature = "spacetimedb-persist")]
    let persist = SpacetimeDbPersist::from_env();

    let interval = Duration::from_millis(1000 / tick_rate_hz);
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
            for entry in delta.updated {
                neighbor_last_seen.insert(entry.entity_id, tick_count);
                neighbor_entities.insert(entry.entity_id, entry);
            }
            for removed_id in &delta.removed {
                neighbor_entities.remove(removed_id);
                neighbor_last_seen.remove(removed_id);
            }
        }
        // Prune stale neighbor entities every ~60 ticks to bound memory from crashed neighbors.
        const PRUNE_INTERVAL_TICKS: u64 = 60;
        if tick_count.is_multiple_of(PRUNE_INTERVAL_TICKS) {
            neighbor_last_seen.retain(|id, last_seen| {
                let keep = tick_count - *last_seen <= NEIGHBOR_STALE_TICKS;
                if !keep {
                    neighbor_entities.remove(id);
                }
                keep
            });
        }
        let mut tick_actions: Vec<GameAction> = Vec::new();
        while let Ok(action) = game_actions_rx.try_recv() {
            tick_actions.push(action);
        }
        // Drain inbound physics events and deliver to the simulation.
        let mut inbound_physics: Vec<PhysicsEventBatch> = Vec::new();
        while let Ok(batch) = physics_events_rx.try_recv() {
            inbound_physics.push(batch);
        }
        if let Some(ref sim) = simulation {
            if !inbound_physics.is_empty() {
                sim.apply_inbound_physics_events(inbound_physics);
            }
        }

        let tick_start = Instant::now();
        let upcoming_tick = server.current_tick() + 1;
        server.simulate_before_tick(
            dt_seconds,
            upcoming_tick,
            simulation.as_ref().map(|s| s.as_ref()),
            &tick_actions,
            &neighbor_entities,
        );

        // Drain routed physics ops and publish to neighbor clusters.
        if let Some(ref sim) = simulation {
            let routed = sim.drain_routed_physics_ops();
            if !routed.is_empty() {
                if let Err(e) = physics_publisher.publish(cluster_id, routed) {
                    eprintln!("physics events publish error: {}", e);
                }
            }
        }

        let our_delta = server.tick();
        let tick_elapsed = tick_start.elapsed();
        let tick_elapsed_ms = tick_elapsed.as_secs_f64() * 1000.0;
        let merged_delta = merge_with_neighbor_latest(our_delta, &neighbor_entities);
        #[cfg(feature = "spacetimedb-persist")]
        if let Some(ref persist) = persist {
            persist.maybe_persist(tick_count, &merged_delta.updated);
        }

        let _ = state_tx.send(merged_delta);

        stats.set_entities(server.entity_count() as u64);
        stats.tick.store(server.current_tick(), Ordering::Relaxed);
        stats
            .seq
            .store(server.current_seq() as u64, Ordering::Relaxed);
        stats
            .last_tick_us
            .store(tick_elapsed.as_micros() as u64, Ordering::Relaxed);

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
                                 // Extended ArcaneServerStats: adds ws_accepts / msgs / parse_failures so log-only
                                 // analysis (no /stats query) can still surface silent failures.
            eprintln!(
                "ArcaneServerStats: entities={} clusters={} tick_ms={:.2} ws_accepts={} msgs_ps={} msgs_ga={} parse_fail={} bytes_in={}",
                entities,
                clusters,
                tick_elapsed_ms,
                stats.ws_accepts.load(Ordering::Relaxed),
                stats.msgs_player_state.load(Ordering::Relaxed),
                stats.msgs_game_action.load(Ordering::Relaxed),
                stats.parse_failures.load(Ordering::Relaxed),
                stats.bytes_in.load(Ordering::Relaxed),
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
        let mut neighbors: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        neighbors.insert(n1_entity.entity_id, n1_entity.clone());
        neighbors.insert(n2_entity.entity_id, n2_entity.clone());

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
        let new_n1_entity = mk_entry(Uuid::from_u128(22), n1, 2.0);

        let mut neighbors: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        neighbors.insert(new_n1_entity.entity_id, new_n1_entity.clone());

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

    #[test]
    fn merge_dedup_local_wins_over_neighbor() {
        let local_cluster = Uuid::from_u128(1);
        let n1 = Uuid::from_u128(2);
        let entity_id = Uuid::from_u128(100);
        let local_entity = mk_entry(entity_id, local_cluster, 10.0);
        let neighbor_entity = mk_entry(entity_id, n1, 20.0);

        let mut neighbors: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        neighbors.insert(entity_id, neighbor_entity);

        let merged = merge_with_neighbor_latest(
            EntityStateDelta {
                source_cluster_id: local_cluster,
                seq: 1,
                tick: 1,
                timestamp: 0.0,
                updated: vec![local_entity.clone()],
                removed: vec![],
            },
            &neighbors,
        );
        assert_eq!(
            merged.updated.len(),
            1,
            "dedup must produce exactly one entry"
        );
        let entry = &merged.updated[0];
        assert_eq!(entry.entity_id, entity_id);
        // Local version wins: position.x should be 10.0, not 20.0
        assert!(
            (entry.position.x - 10.0).abs() < 1e-6,
            "local position must win, got {}",
            entry.position.x
        );
    }

    #[test]
    fn neighbor_removed_entity_leaves_map() {
        let entity_id = Uuid::from_u128(200);
        let entity = mk_entry(entity_id, Uuid::from_u128(2), 15.0);
        let delta_add = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(2),
            seq: 1,
            tick: 1,
            timestamp: 0.0,
            updated: vec![entity.clone()],
            removed: vec![],
        };
        let delta_remove = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(2),
            seq: 2,
            tick: 2,
            timestamp: 0.0,
            updated: vec![],
            removed: vec![entity_id],
        };

        let mut neighbor_entities: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        let mut neighbor_last_seen: HashMap<Uuid, u64> = HashMap::new();
        let mut tick_count: u64 = 0;

        // Apply add delta (simulating the drain loop logic)
        tick_count += 1;
        for entry in &delta_add.updated {
            neighbor_last_seen.insert(entry.entity_id, tick_count);
            neighbor_entities.insert(entry.entity_id, entry.clone());
        }
        assert!(neighbor_entities.contains_key(&entity_id));

        // Apply remove delta
        tick_count += 1;
        for removed_id in &delta_remove.removed {
            neighbor_entities.remove(removed_id);
            neighbor_last_seen.remove(removed_id);
        }
        for entry in &delta_remove.updated {
            neighbor_last_seen.insert(entry.entity_id, tick_count);
            neighbor_entities.insert(entry.entity_id, entry.clone());
        }
        assert!(!neighbor_entities.contains_key(&entity_id));
    }

    #[test]
    fn neighbor_entity_survives_missing_from_later_delta() {
        let entity_id = Uuid::from_u128(300);
        let entity = mk_entry(entity_id, Uuid::from_u128(2), 25.0);
        let delta_1 = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(2),
            seq: 1,
            tick: 1,
            timestamp: 0.0,
            updated: vec![entity.clone()],
            removed: vec![],
        };
        // Delta 2 does NOT mention entity_id (dead reckoning omission)
        let delta_2 = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(2),
            seq: 2,
            tick: 2,
            timestamp: 0.0,
            updated: vec![],
            removed: vec![],
        };

        let mut neighbor_entities: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        let mut neighbor_last_seen: HashMap<Uuid, u64> = HashMap::new();
        let mut tick_count: u64 = 0;

        tick_count += 1;
        for entry in &delta_1.updated {
            neighbor_last_seen.insert(entry.entity_id, tick_count);
            neighbor_entities.insert(entry.entity_id, entry.clone());
        }
        assert!(neighbor_entities.contains_key(&entity_id));

        tick_count += 1;
        for entry in &delta_2.updated {
            neighbor_last_seen.insert(entry.entity_id, tick_count);
            neighbor_entities.insert(entry.entity_id, entry.clone());
        }
        // Entity must survive — the map persists entries across ticks
        assert!(neighbor_entities.contains_key(&entity_id));
    }

    #[test]
    fn neighbor_entities_accumulate_across_deltas() {
        let e1 = mk_entry(Uuid::from_u128(401), Uuid::from_u128(2), 1.0);
        let e2 = mk_entry(Uuid::from_u128(402), Uuid::from_u128(3), 2.0);
        let delta_1 = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(2),
            seq: 1,
            tick: 1,
            timestamp: 0.0,
            updated: vec![e1.clone()],
            removed: vec![],
        };
        let delta_2 = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(3),
            seq: 1,
            tick: 1,
            timestamp: 0.0,
            updated: vec![e2.clone()],
            removed: vec![],
        };

        let mut neighbor_entities: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        let mut neighbor_last_seen: HashMap<Uuid, u64> = HashMap::new();

        for delta in &[delta_1, delta_2] {
            for entry in &delta.updated {
                neighbor_last_seen.insert(entry.entity_id, 1);
                neighbor_entities.insert(entry.entity_id, entry.clone());
            }
        }
        assert_eq!(neighbor_entities.len(), 2);
        assert!(neighbor_entities.contains_key(&e1.entity_id));
        assert!(neighbor_entities.contains_key(&e2.entity_id));
    }
}
