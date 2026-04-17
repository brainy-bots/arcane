//! ClusterServer (IN-02) — simulation unit per cluster.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arcane_core::cluster_simulation::{ClusterSimulation, ClusterTickContext, GameAction};
use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
use uuid::Uuid;

use crate::ReplicationChannelManager;

/// Default maximum number of entities a single cluster will hold. Prevents unbounded memory
/// growth from misbehaving clients sending unique entity IDs. The cap applies to `add_entity`;
/// entities injected by `simulate_before_tick` are not capped (they are server-authoritative).
pub const DEFAULT_MAX_ENTITIES: usize = 100_000;

/// One process per cluster. Runs simulation, replication, client connections.
pub struct ClusterServer {
    cluster_id: Uuid,
    tick: AtomicU64,
    seq: AtomicI64,
    replication: Mutex<Option<Arc<ReplicationChannelManager>>>,
    entities: Mutex<HashMap<Uuid, EntityStateEntry>>,
    pending_removed: Mutex<Vec<Uuid>>,
    max_entities: usize,
}

impl ClusterServer {
    pub fn new(cluster_id: Uuid) -> Self {
        Self::with_max_entities(cluster_id, DEFAULT_MAX_ENTITIES)
    }

    pub fn with_max_entities(cluster_id: Uuid, max_entities: usize) -> Self {
        Self {
            cluster_id,
            tick: AtomicU64::new(0),
            seq: AtomicI64::new(0),
            replication: Mutex::new(None),
            entities: Mutex::new(HashMap::new()),
            pending_removed: Mutex::new(Vec::new()),
            max_entities,
        }
    }

    /// Attach replication manager. Call after start(redis_url) and set_neighbors on the manager.
    pub fn set_replication(&self, mgr: Arc<ReplicationChannelManager>) {
        let mut guard = self.replication.lock().expect("replication lock");
        *guard = Some(mgr);
    }

    /// Add or update an entity in this cluster's local state. Included in next tick's delta.
    /// Silently drops the entry if the entity map is at capacity and the entity_id is new
    /// (updates to existing entities are always accepted).
    pub fn add_entity(&self, entry: EntityStateEntry) {
        let mut entities = self.entities.lock().expect("entities lock");
        if entities.len() >= self.max_entities && !entities.contains_key(&entry.entity_id) {
            return;
        }
        entities.insert(entry.entity_id, entry);
    }

    /// Mark an entity for removal. It will appear in the next tick's delta as removed, then be dropped from local state.
    pub fn remove_entity(&self, entity_id: Uuid) {
        let mut entities = self.entities.lock().expect("entities lock");
        entities.remove(&entity_id);
        self.pending_removed
            .lock()
            .expect("pending_removed lock")
            .push(entity_id);
    }

    /// Runs custom simulation with exclusive access to the local entity map, then applies any
    /// [`ClusterTickContext::pending_removals`]. Call immediately before [`ClusterServer::tick`].
    /// `upcoming_tick` must match the tick index the next `tick()` will assign (`current_tick() + 1`
    /// before the first `tick()` call).
    ///
    /// `game_actions` contains client actions received since the last tick. The simulation
    /// decides how to handle them (e.g., validate through SpacetimeDB, apply buffs).
    pub fn simulate_before_tick(
        &self,
        dt_seconds: f64,
        upcoming_tick: u64,
        simulation: Option<&dyn ClusterSimulation>,
        game_actions: &[GameAction],
    ) {
        let Some(sim) = simulation else {
            return;
        };
        let mut pending_removals = Vec::new();
        {
            let mut entities = self.entities.lock().expect("entities lock");
            sim.on_tick(&mut ClusterTickContext {
                cluster_id: self.cluster_id,
                tick: upcoming_tick,
                dt_seconds,
                entities: &mut entities,
                pending_removals: &mut pending_removals,
                game_actions,
            });
        }
        for id in pending_removals {
            self.remove_entity(id);
        }
    }

    /// Advance simulation by one tick, build delta from current entities, broadcast to neighbors if set, and return the delta.
    pub fn tick(&self) -> EntityStateDelta {
        let t = self.tick.fetch_add(1, Ordering::Relaxed) + 1;
        let s = self.seq.fetch_add(1, Ordering::Relaxed) + 1;

        let (updated, removed) = {
            let entities = self.entities.lock().expect("entities lock");
            let updated: Vec<EntityStateEntry> = entities.values().cloned().collect();
            let mut pending = self.pending_removed.lock().expect("pending_removed lock");
            let removed = std::mem::take(&mut *pending);
            (updated, removed)
        };

        let delta = EntityStateDelta {
            source_cluster_id: self.cluster_id,
            seq: s,
            tick: t,
            timestamp: 0.0,
            updated,
            removed,
        };

        let guard = self.replication.lock().expect("replication lock");
        if let Some(ref mgr) = *guard {
            if mgr.channel_count() > 0 {
                mgr.send_to_neighbors(delta.clone());
            }
        }

        delta
    }

    /// Current tick number (for tests / metrics).
    pub fn current_tick(&self) -> u64 {
        self.tick.load(Ordering::Relaxed)
    }

    /// Current replication sequence (for tests / metrics).
    pub fn current_seq(&self) -> i64 {
        self.seq.load(Ordering::Relaxed)
    }

    /// Number of entities in this cluster (for server load / benchmark visibility).
    pub fn entity_count(&self) -> usize {
        self.entities.lock().expect("entities lock").len()
    }

    pub fn cluster_id(&self) -> Uuid {
        self.cluster_id
    }
}
