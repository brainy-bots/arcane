//! ArcaneNode (IN-02) — simulation unit per cluster.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use arcane_core::cluster_simulation::{ClusterSimulation, ClusterTickContext, GameAction};
use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
use arcane_wire::Vec3Q;
use uuid::Uuid;

use crate::ReplicationChannelManager;

/// Default maximum number of entities a single cluster will hold. Prevents unbounded memory
/// growth from misbehaving clients sending unique entity IDs. The cap applies to `add_entity`;
/// entities injected by `simulate_before_tick` are not capped (they are server-authoritative).
pub const DEFAULT_MAX_ENTITIES: usize = 100_000;

/// Default resync cadence for velocity-based dead reckoning, in ticks. Every
/// N ticks the cluster broadcasts every entity regardless of velocity change,
/// so clients that missed a velocity-change broadcast (packet loss, late
/// join) re-anchor to fresh server state. 60 ticks ≈ 2-3 sec wall-clock
/// across the benchmark's tick range (20-60 Hz). Override via
/// `ARCANE_RESYNC_EVERY_N_TICKS`.
pub const DEFAULT_RESYNC_EVERY_N_TICKS: u64 = 60;

/// Read the resync cadence from the environment. Clamped to `>= 1` so a
/// misconfigured value can't accidentally disable resync entirely (which
/// would leave dropped-velocity-change entities stale forever).
fn resolve_resync_every_n_ticks() -> u64 {
    std::env::var("ARCANE_RESYNC_EVERY_N_TICKS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_RESYNC_EVERY_N_TICKS)
}

/// One process per cluster. Runs simulation, replication, client connections.
pub struct ArcaneNode {
    cluster_id: Uuid,
    tick: AtomicU64,
    seq: AtomicI64,
    replication: Mutex<Option<Arc<ReplicationChannelManager>>>,
    entities: Mutex<HashMap<Uuid, EntityStateEntry>>,
    pending_removed: Mutex<Vec<Uuid>>,
    max_entities: usize,
    /// Last-broadcast velocity per entity (quantized to wire form). Used for
    /// velocity-based dead reckoning: an entity is omitted from a broadcast
    /// when its current velocity quantizes identically to its last-broadcast
    /// velocity. New entities and the periodic resync tick force inclusion.
    /// Comparing in `Vec3Q` (i16) instead of `Vec3` (f64) means the skip
    /// decision matches the client's view exactly — if the wire bytes
    /// wouldn't change, the broadcast doesn't happen.
    last_broadcast_velocity: Mutex<HashMap<Uuid, Vec3Q>>,
    /// Resync cadence in ticks. Read once at construction from
    /// `ARCANE_RESYNC_EVERY_N_TICKS`. See [`DEFAULT_RESYNC_EVERY_N_TICKS`].
    resync_every_n_ticks: u64,
}

impl ArcaneNode {
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
            last_broadcast_velocity: Mutex::new(HashMap::new()),
            resync_every_n_ticks: resolve_resync_every_n_ticks(),
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
    /// [`ClusterTickContext::pending_removals`]. Call immediately before [`ArcaneNode::tick`].
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
        neighbor_entities: &HashMap<Uuid, EntityStateEntry>,
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
                neighbor_entities,
            });
        }
        for id in pending_removals {
            self.remove_entity(id);
        }
    }

    /// Advance simulation by one tick, build delta from current entities, broadcast to neighbors if set, and return the delta.
    ///
    /// **Velocity-based dead reckoning.** Entities whose current velocity
    /// quantizes identically to their last-broadcast velocity are omitted
    /// from the `updated` list (clients hold the last anchor and extrapolate
    /// position locally via `pos(t) = pos_last + vel_last × (t − t_last)`).
    /// First-broadcast entities and the periodic resync tick force inclusion
    /// so packet-loss / late-join scenarios converge. The `removed` list is
    /// always carried verbatim — removals can't be inferred client-side.
    pub fn tick(&self) -> EntityStateDelta {
        let t = self.tick.fetch_add(1, Ordering::Relaxed) + 1;
        let s = self.seq.fetch_add(1, Ordering::Relaxed) + 1;

        // A resync tick rebroadcasts every entity (both for late joiners and
        // to recover from any silently dropped velocity-change broadcasts).
        // Tick 0 is impossible here (we just incremented), so the first
        // resync naturally fires at tick `resync_every_n_ticks` rather than
        // on the very first tick — that's fine, the first-broadcast path
        // below already includes every entity once.
        let is_resync_tick = t.is_multiple_of(self.resync_every_n_ticks);

        let (updated, removed) = {
            let entities = self.entities.lock().expect("entities lock");
            let mut last_vel = self
                .last_broadcast_velocity
                .lock()
                .expect("last_broadcast_velocity lock");

            // Collect entities that need broadcasting this tick. New entity
            // (no last-broadcast record), velocity-quantum-changed entity,
            // or every entity on a resync tick — otherwise skip and let the
            // client extrapolate from its last anchor.
            let mut updated: Vec<EntityStateEntry> = Vec::new();
            for entry in entities.values() {
                let current_vel_q = Vec3Q::from_vec3(arcane_wire::Vec3::new(
                    entry.velocity.x,
                    entry.velocity.y,
                    entry.velocity.z,
                ));
                let include = match last_vel.get(&entry.entity_id) {
                    None => true, // new entity — first broadcast
                    Some(_) if is_resync_tick => true,
                    Some(prev) => *prev != current_vel_q,
                };
                if include {
                    last_vel.insert(entry.entity_id, current_vel_q);
                    updated.push(entry.clone());
                }
            }

            let mut pending = self.pending_removed.lock().expect("pending_removed lock");
            let removed = std::mem::take(&mut *pending);
            // Drop the dead-reckoning record for entities that just left so
            // the map stays bounded by `entity_count`, not lifetime-unique
            // ids ever seen.
            for id in &removed {
                last_vel.remove(id);
            }
            (updated, removed)
        };

        // Wall-clock UNIX seconds at the moment the cluster produced this
        // delta. Driver-side latency decomposition uses this as T2 (the
        // server-stamped point on the timeline) so it can split the existing
        // T3 - T1 client-perceived latency into a wire portion (T2 - T1) and
        // a driver-processing portion. EC2 instances are chrony-synced to
        // ~1ms, well below the 200ms latency budget; the cross-clock noise
        // is acceptable for diagnosis. Falls back to 0.0 if the system
        // clock is somehow before UNIX_EPOCH (impossible on AWS in practice).
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let delta = EntityStateDelta {
            source_cluster_id: self.cluster_id,
            seq: s,
            tick: t,
            timestamp,
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
