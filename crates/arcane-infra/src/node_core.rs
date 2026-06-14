//! Node core — embeddable core that drives one iteration of the node loop.
//!
//! This module extracts the reusable `NodeCore` from `run_node_loop`, leaving the loop
//! ownership and timing to the driver. `NodeCore::new()` runs all setup (Redis start,
//! channel creation, I/O thread spawning); `NodeCore::tick()` executes one iteration of
//! the loop body (drain inputs, simulate, tick, broadcast).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::atomic::Ordering;

use arcane_core::cluster_simulation::ClusterSimulation;
use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
use uuid::Uuid;

#[cfg(feature = "cluster-ws")]
use crate::neighbor_subscriber::spawn_neighbor_subscriber;
#[cfg(feature = "cluster-ws")]
use crate::node_stats::NodeStats;
#[cfg(feature = "cluster-ws")]
use crate::physics_events_channel::{spawn_physics_events_subscriber, PhysicsEventsPublisher};
#[cfg(feature = "spacetimedb-persist")]
use crate::spacetimedb_persist::SpacetimeDbPersist;
use crate::{ArcaneNode, ReplicationChannelManager};

const LOG_EVERY_TICKS: u64 = 100;
const LOG_STATS_EVERY_TICKS: u64 = 40;
const NEIGHBOR_STALE_TICKS: u64 = 300;

/// Configuration for creating a `NodeCore`.
#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub cluster_id: Uuid,
    pub redis_url: String,
    pub neighbor_ids: Vec<Uuid>,
    pub ws_port: u16,
}

/// The core node state machine — all components except loop ownership and timing.
///
/// `NodeCore` owns the `ArcaneNode`, replication manager, all channel endpoints,
/// physics publisher, neighbor entity tracking, stats, and persistence. The driver
/// (`run_node_loop`) owns the `loop {}`, interval, and `thread::sleep`.
pub struct NodeCore {
    server: ArcaneNode,
    state_tx: std::sync::mpsc::Sender<EntityStateDelta>,
    client_updates_rx: std::sync::mpsc::Receiver<EntityStateEntry>,
    game_actions_rx: std::sync::mpsc::Receiver<arcane_core::cluster_simulation::GameAction>,
    neighbor_rx: std::sync::mpsc::Receiver<EntityStateDelta>,
    neighbor_entities: HashMap<Uuid, EntityStateEntry>,
    neighbor_last_seen: HashMap<Uuid, u64>,
    physics_events_rx: std::sync::mpsc::Receiver<arcane_core::physics_events::PhysicsEventBatch>,
    physics_publisher: PhysicsEventsPublisher,
    stats: Arc<NodeStats>,
    tick_count: u64,
    cluster_id: Uuid,
    dt_seconds: f64,
    #[cfg(feature = "spacetimedb-persist")]
    persist: Option<SpacetimeDbPersist>,
}

impl NodeCore {
    /// Initialize the node core: Redis start, replication setup, channel creation,
    /// I/O thread spawning. Returns Err on setup failure (Redis, physics publisher).
    pub fn new(cfg: NodeConfig) -> Result<Self, String> {
        let replication = ReplicationChannelManager::new(cfg.cluster_id);
        replication
            .start(&cfg.redis_url)
            .map_err(|e| format!("Redis start failed: {}", e))?;
        replication.set_neighbors(cfg.neighbor_ids.clone());

        let server = ArcaneNode::new(cfg.cluster_id);
        server.set_replication(Arc::new(replication));

        let (state_tx, state_rx) = std::sync::mpsc::channel();
        let (client_updates_tx, client_updates_rx) = std::sync::mpsc::channel();
        let (game_actions_tx, game_actions_rx) = std::sync::mpsc::channel();

        let stats = NodeStats::new();
        let stats_port = std::env::var("NODE_STATS_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(cfg.ws_port.saturating_add(1));
        crate::node_stats::serve_stats_http(stats_port, cfg.cluster_id.to_string(), stats.clone());

        crate::ws_server::run_ws_server(
            cfg.ws_port,
            state_rx,
            client_updates_tx,
            game_actions_tx,
            stats.clone(),
        );

        let (neighbor_tx, neighbor_rx) = std::sync::mpsc::channel();
        spawn_neighbor_subscriber(cfg.redis_url.clone(), cfg.neighbor_ids.clone(), neighbor_tx);

        let (physics_events_tx, physics_events_rx) = std::sync::mpsc::channel();
        spawn_physics_events_subscriber(cfg.redis_url.clone(), cfg.cluster_id, physics_events_tx);
        let physics_publisher = PhysicsEventsPublisher::new(&cfg.redis_url)
            .map_err(|e| format!("physics events publisher: {}", e))?;

        let tick_rate_hz = crate::tick_rate::tick_rate_hz();
        eprintln!(
            "arcane-node started cluster_id={} neighbors={} tick_rate={}Hz",
            cfg.cluster_id,
            cfg.neighbor_ids.len(),
            tick_rate_hz
        );

        #[cfg(feature = "spacetimedb-persist")]
        let persist = SpacetimeDbPersist::from_env();
        #[cfg(not(feature = "spacetimedb-persist"))]
        let _persist = ();

        let interval = Duration::from_millis(1000 / tick_rate_hz);
        let dt_seconds = interval.as_secs_f64();

        Ok(Self {
            server,
            state_tx,
            client_updates_rx,
            game_actions_rx,
            neighbor_rx,
            neighbor_entities: HashMap::new(),
            neighbor_last_seen: HashMap::new(),
            physics_events_rx,
            physics_publisher,
            stats: stats.clone(),
            tick_count: 0,
            cluster_id: cfg.cluster_id,
            dt_seconds,
            #[cfg(feature = "spacetimedb-persist")]
            persist,
        })
    }

    /// Execute one iteration of the node loop body: drain inputs, simulate, tick, broadcast.
    /// Does NOT sleep or own the loop. `tick_count` is pre-incremented (current iteration's count
    /// before the increment) — this matches the existing semantics where logging and neighbor
    /// bookkeeping use the pre-increment value.
    ///
    /// ⚠️ **TRANSITIONAL**: This method's signature and semantics are not frozen. The `submit/pump/drain`
    /// surface introduced in sub-issue #2 will replace this interface and resolve where `ClusterSimulation`
    /// runs (today it runs inside `ArcaneNode::simulate_before_tick`; ownership may shift).
    /// For this sub-issue, keeping the sim call inside `tick` preserves today's behavior exactly.
    pub fn tick(
        &mut self,
        simulation: Option<&dyn ClusterSimulation>,
        extra_entities: Vec<EntityStateEntry>,
    ) {
        while let Ok(mut entry) = self.client_updates_rx.try_recv() {
            entry.cluster_id = self.cluster_id;
            self.server.add_entity(entry);
        }
        for mut entry in extra_entities {
            entry.cluster_id = self.cluster_id;
            self.server.add_entity(entry);
        }
        while let Ok(delta) = self.neighbor_rx.try_recv() {
            for entry in delta.updated {
                self.neighbor_last_seen
                    .insert(entry.entity_id, self.tick_count);
                self.neighbor_entities.insert(entry.entity_id, entry);
            }
            for removed_id in &delta.removed {
                self.neighbor_entities.remove(removed_id);
                self.neighbor_last_seen.remove(removed_id);
            }
        }
        const PRUNE_INTERVAL_TICKS: u64 = 60;
        if self.tick_count.is_multiple_of(PRUNE_INTERVAL_TICKS) {
            self.neighbor_last_seen.retain(|id, last_seen| {
                let keep = self.tick_count - *last_seen <= NEIGHBOR_STALE_TICKS;
                if !keep {
                    self.neighbor_entities.remove(id);
                }
                keep
            });
        }
        let mut tick_actions = Vec::new();
        while let Ok(action) = self.game_actions_rx.try_recv() {
            tick_actions.push(action);
        }
        let mut inbound_physics = Vec::new();
        while let Ok(batch) = self.physics_events_rx.try_recv() {
            inbound_physics.push(batch);
        }
        if let Some(sim) = simulation {
            if !inbound_physics.is_empty() {
                sim.apply_inbound_physics_events(inbound_physics);
            }
        }

        let tick_start = Instant::now();
        let upcoming_tick = self.server.current_tick() + 1;
        self.server.simulate_before_tick(
            self.dt_seconds,
            upcoming_tick,
            simulation,
            &tick_actions,
            &self.neighbor_entities,
        );

        if let Some(sim) = simulation {
            let routed = sim.drain_routed_physics_ops();
            if !routed.is_empty() {
                if let Err(e) = self.physics_publisher.publish(self.cluster_id, routed) {
                    eprintln!("physics events publish error: {}", e);
                }
            }
        }

        let our_delta = self.server.tick();
        let tick_elapsed = tick_start.elapsed();
        let tick_elapsed_ms = tick_elapsed.as_secs_f64() * 1000.0;
        let merged_delta = merge_with_neighbor_latest(our_delta, &self.neighbor_entities);
        #[cfg(feature = "spacetimedb-persist")]
        if let Some(ref persist) = self.persist {
            persist.maybe_persist(self.tick_count, &merged_delta.updated);
        }

        let _ = self.state_tx.send(merged_delta);

        self.stats.set_entities(self.server.entity_count() as u64);
        self.stats
            .tick
            .store(self.server.current_tick(), Ordering::Relaxed);
        self.stats
            .seq
            .store(self.server.current_seq() as u64, Ordering::Relaxed);
        self.stats
            .last_tick_us
            .store(tick_elapsed.as_micros() as u64, Ordering::Relaxed);

        self.tick_count += 1;
        if self.tick_count.is_multiple_of(LOG_EVERY_TICKS) {
            eprintln!(
                "tick {} seq {}",
                self.server.current_tick(),
                self.server.current_seq()
            );
        }
        if self.tick_count.is_multiple_of(LOG_STATS_EVERY_TICKS) {
            let entities = self.server.entity_count();
            let clusters = 1u32;
            eprintln!(
                "ArcaneServerStats: entities={} clusters={} tick_ms={:.2} ws_accepts={} msgs_ps={} msgs_ga={} parse_fail={} bytes_in={} bytes_out={} lagged_events={} lagged_frames={} send_err={}",
                entities,
                clusters,
                tick_elapsed_ms,
                self.stats.ws_accepts.load(Ordering::Relaxed),
                self.stats.msgs_player_state.load(Ordering::Relaxed),
                self.stats.msgs_game_action.load(Ordering::Relaxed),
                self.stats.parse_failures.load(Ordering::Relaxed),
                self.stats.bytes_in.load(Ordering::Relaxed),
                self.stats.bytes_out.load(Ordering::Relaxed),
                self.stats.broadcast_lagged_events.load(Ordering::Relaxed),
                self.stats.broadcast_lagged_frames.load(Ordering::Relaxed),
                self.stats.ws_send_errors.load(Ordering::Relaxed),
            );
        }
    }

    /// Current tick count (pre-increment value). The driver calls `extra_entities_for_tick(tick_count())`
    /// before `tick()` to allow the driver to generate entities for this iteration.
    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }
}

/// Merge local delta with latest neighbor snapshots, deduplicating on entity_id
/// (local entries win). Used in `NodeCore::tick()` and exposed for tests.
pub fn merge_with_neighbor_latest(
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

        tick_count += 1;
        for entry in &delta_add.updated {
            neighbor_last_seen.insert(entry.entity_id, tick_count);
            neighbor_entities.insert(entry.entity_id, entry.clone());
        }
        assert!(neighbor_entities.contains_key(&entity_id));

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
