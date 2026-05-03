//! Rapier-backed authoritative physics for the cluster tick.
//!
//! [`RapierClusterSim`] wraps a user's [`ClusterSimulation`] and inserts a Rapier
//! [`PhysicsPipeline::step`] after the user's `on_tick`. Drop into the existing
//! [`crate::cluster_runner::run_cluster_loop`] in place of a bare user simulation —
//! all networking, replication, neighbor merge, and persistence are unchanged.
//!
//! # Contract
//!
//! - `entity.velocity` is **intent-in**. The user's `on_tick` writes it; Rapier reads
//!   it as the rigid body's `linvel` for the upcoming step.
//! - `entity.position` is **output-only after first-sight spawn**. The first time an
//!   entity appears in the entity map, its position seeds the rigid body's translation.
//!   Subsequent user writes are overwritten by Rapier's post-step output.
//! - Despawn is driven by `pending_removals` — when an entity leaves the map, its
//!   rigid body and collider are removed from the Rapier world.
//! - Default collider is a uniform sphere ([`RapierConfig::default_body_radius`]).
//!   Implement [`RapierClusterSimulation`] and override `collider_for` to declare
//!   per-entity shapes ([`RapierColliderShape::Ball`] / `Capsule` / `Cuboid`).
//!   Shape is fixed at first-sight spawn; later `collider_for` returns are ignored
//!   for already-spawned entities (despawn-and-respawn to change shape).
//!
//! # Contact events
//!
//! [`RapierClusterSimulation::on_tick`] receives a [`RapierClusterTickContext`]
//! carrying `contact_events: &[ContactEvent]` — collisions detected during the
//! **previous** tick's physics step, with both entity ids and a `started` flag.
//! One-tick delay is by design: user logic runs first to set intent, then physics
//! produces output for next tick. Same-tick post-physics reactivity is a future
//! follow-up.
//!
//! # Substepping
//!
//! The cluster tick is variable (env-driven, default 20 Hz). Rapier prefers fixed
//! substeps for stability. We accumulate `dt_seconds` and step Rapier in
//! [`FIXED_PHYSICS_DT`]-sized increments until the accumulator drains.
//!
//! # Precision
//!
//! `EntityStateEntry` uses `f64` positions and velocities; Rapier uses `f32`
//! internally. Conversion happens on every input/output sync. For worlds within
//! ~10⁴ units of origin this is sub-millimeter; far-from-origin coordinates lose
//! precision in the standard `f32` way. If your world exceeds those bounds,
//! enable Rapier's `f64` feature in a follow-up.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use arcane_core::ClusterSimulation;
//! use arcane_infra::{RapierClusterSim, RapierConfig};
//!
//! // Pure-Rapier: no game logic, just integrate velocity into position.
//! let physics: Arc<dyn ClusterSimulation> =
//!     Arc::new(RapierClusterSim::new(None, RapierConfig::default()));
//!
//! // Or: wrap your own ClusterSimulation so user logic runs first, then Rapier.
//! // let user_sim: Arc<dyn ClusterSimulation> = Arc::new(MyGameLogic::new());
//! // let physics = Arc::new(RapierClusterSim::new(Some(user_sim), RapierConfig::default()));
//!
//! // Or: implement RapierClusterSimulation for per-entity shapes + contact events:
//! // let game: Arc<dyn arcane_infra::RapierClusterSimulation> = Arc::new(MyGame::new());
//! // let physics = Arc::new(RapierClusterSim::with_rapier_sim(game, RapierConfig::default()));
//!
//! // Pass `Some(physics)` as the simulation arg to `run_cluster_loop`.
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rapier3d::prelude::*;
use uuid::Uuid;

use arcane_core::cluster_simulation::{ClusterSimulation, ClusterTickContext};
use arcane_core::replication_channel::EntityStateEntry;

/// Fixed Rapier substep size. 1/60 s matches the standard physics rate.
const FIXED_PHYSICS_DT: f32 = 1.0 / 60.0;

/// V1 default body shape — uniform sphere collider. Per-entity shapes via `user_data`
/// schema is a follow-up.
const DEFAULT_BODY_RADIUS: f32 = 0.5;

/// Configuration knobs for [`RapierClusterSim`].
#[derive(Clone, Debug)]
pub struct RapierConfig {
    /// World gravity vector in m/s². Default is zero gravity (matches benchmark
    /// parity: today's benchmark cluster does pure velocity integration with no
    /// downward acceleration). Set to e.g. `[0.0, -9.81, 0.0]` for Earth gravity
    /// along -Y.
    pub gravity: [f32; 3],
    /// Sphere collider radius applied to every spawned entity. v1 uses one shape
    /// for all bodies; per-entity shapes are a follow-up.
    pub default_body_radius: f32,
}

impl Default for RapierConfig {
    fn default() -> Self {
        Self {
            gravity: [0.0, 0.0, 0.0],
            default_body_radius: DEFAULT_BODY_RADIUS,
        }
    }
}

/// The collider shape used for an entity's rigid body. Resolved at first-sight
/// spawn via [`RapierClusterSimulation::collider_for`]; subsequent calls are
/// ignored for already-spawned entities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RapierColliderShape {
    /// Sphere with the given radius.
    Ball(f32),
    /// Capsule oriented along Y, defined by the half-length of the cylindrical
    /// section (excluding the hemispherical caps) and the radius.
    Capsule {
        /// Half-length of the cylindrical mid-section along Y.
        half_height: f32,
        /// Radius of the capsule (also the radius of the hemispherical caps).
        radius: f32,
    },
    /// Axis-aligned box defined by half-extents along each axis (X, Y, Z).
    Cuboid([f32; 3]),
}

impl Default for RapierColliderShape {
    fn default() -> Self {
        RapierColliderShape::Ball(DEFAULT_BODY_RADIUS)
    }
}

fn build_collider(shape: RapierColliderShape) -> Collider {
    let builder = match shape {
        RapierColliderShape::Ball(radius) => ColliderBuilder::ball(radius),
        RapierColliderShape::Capsule {
            half_height,
            radius,
        } => ColliderBuilder::capsule_y(half_height, radius),
        RapierColliderShape::Cuboid(he) => ColliderBuilder::cuboid(he[0], he[1], he[2]),
    };
    builder.active_events(ActiveEvents::COLLISION_EVENTS).build()
}

/// A collision detected during a Rapier step, mapped from Rapier's collider
/// handles back to entity ids. Surfaced to [`RapierClusterSimulation::on_tick`]
/// via [`RapierClusterTickContext::contact_events`].
///
/// `started == true` signals the contact started this tick; `false` signals it
/// stopped. Stop events also fire when one of the colliders is destroyed (e.g.,
/// despawn / pending_removal), in which case the corresponding entity_id is the
/// one that was just removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContactEvent {
    pub entity_a: Uuid,
    pub entity_b: Uuid,
    pub started: bool,
}

/// Tick context delivered to [`RapierClusterSimulation::on_tick`]. Mirrors
/// [`ClusterTickContext`] field-for-field plus Rapier-specific extensions.
pub struct RapierClusterTickContext<'a> {
    pub cluster_id: Uuid,
    pub tick: u64,
    pub dt_seconds: f64,
    pub entities: &'a mut HashMap<Uuid, EntityStateEntry>,
    pub pending_removals: &'a mut Vec<Uuid>,
    pub game_actions: &'a [arcane_core::cluster_simulation::GameAction],
    /// Contact events from the **previous** tick's physics step. One-tick delay
    /// by design — see module-level "Contact events" docs.
    pub contact_events: &'a [ContactEvent],
}

/// Rapier-aware sibling of [`ClusterSimulation`]. Implement this trait and pass
/// the impl to [`RapierClusterSim::with_rapier_sim`] when you need per-entity
/// collider shapes or contact events. For pure velocity-integration with no
/// shape customization, the plain [`ClusterSimulation`] path remains valid.
pub trait RapierClusterSimulation: Send + Sync {
    /// Per-tick hook. Same lifecycle as [`ClusterSimulation::on_tick`] —
    /// runs after client updates and before the Rapier physics step.
    fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>);

    /// Declare the collider shape for an entity at first-sight spawn. Default
    /// returns `Ball(config.default_body_radius)` — same as the V1 path.
    /// Override to vary shape per entity (read `entry.user_data` for class
    /// info, etc.). Called exactly once per entity, when its body is first
    /// created in the Rapier world.
    fn collider_for(
        &self,
        _entry: &EntityStateEntry,
        config: &RapierConfig,
    ) -> RapierColliderShape {
        RapierColliderShape::Ball(config.default_body_radius)
    }
}

struct RapierState {
    bodies: RigidBodySet,
    colliders: ColliderSet,
    integration_parameters: IntegrationParameters,
    physics_pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    handles: HashMap<Uuid, RigidBodyHandle>,
    /// Reverse map for translating Rapier's `CollisionEvent` collider handles
    /// back into entity ids. Maintained alongside `handles` on every spawn /
    /// despawn.
    collider_to_entity: HashMap<ColliderHandle, Uuid>,
    accumulator: f32,
    gravity: Vector,
    /// Contact events accumulated during the most recent `step_with_accumulator`
    /// call. The wrapper drains these at the top of the next tick and surfaces
    /// them to the user via [`RapierClusterTickContext::contact_events`].
    pending_contact_events: Vec<ContactEvent>,
}

/// Internal `EventHandler` impl that records collisions into a `Mutex<Vec>`.
/// We only use `handle_collision_event` in v2; force events are out of scope.
struct CollisionRecorder {
    events: Mutex<Vec<CollisionEvent>>,
}

impl CollisionRecorder {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    fn drain(self) -> Vec<CollisionEvent> {
        self.events.into_inner().unwrap_or_default()
    }
}

impl EventHandler for CollisionRecorder {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        if let Ok(mut buf) = self.events.lock() {
            buf.push(event);
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: Real,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: Real,
    ) {
    }
}

impl RapierState {
    fn new(gravity: Vector) -> Self {
        Self {
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            integration_parameters: IntegrationParameters {
                dt: FIXED_PHYSICS_DT,
                ..IntegrationParameters::default()
            },
            physics_pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            handles: HashMap::new(),
            collider_to_entity: HashMap::new(),
            accumulator: 0.0,
            gravity,
            pending_contact_events: Vec::new(),
        }
    }

    fn spawn(
        &mut self,
        entity_id: Uuid,
        entry: &EntityStateEntry,
        shape: RapierColliderShape,
    ) -> RigidBodyHandle {
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(
                entry.position.x as f32,
                entry.position.y as f32,
                entry.position.z as f32,
            ))
            .linvel(Vector::new(
                entry.velocity.x as f32,
                entry.velocity.y as f32,
                entry.velocity.z as f32,
            ))
            .build();
        let body_handle = self.bodies.insert(body);
        let collider = build_collider(shape);
        let collider_handle =
            self.colliders
                .insert_with_parent(collider, body_handle, &mut self.bodies);
        self.handles.insert(entity_id, body_handle);
        self.collider_to_entity.insert(collider_handle, entity_id);
        body_handle
    }

    fn set_linvel(&mut self, entity_id: Uuid, vel_x: f64, vel_y: f64, vel_z: f64) {
        let Some(&handle) = self.handles.get(&entity_id) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return;
        };
        body.set_linvel(Vector::new(vel_x as f32, vel_y as f32, vel_z as f32), true);
    }

    fn despawn(&mut self, entity_id: Uuid) {
        let Some(body_handle) = self.handles.remove(&entity_id) else {
            return;
        };
        // Drop reverse-map entries for any colliders attached to this body.
        if let Some(body) = self.bodies.get(body_handle) {
            let coll_handles: Vec<ColliderHandle> = body.colliders().to_vec();
            for ch in coll_handles {
                self.collider_to_entity.remove(&ch);
            }
        }
        self.bodies.remove(
            body_handle,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
    }

    fn step_with_accumulator(&mut self, dt_seconds: f32) {
        self.accumulator += dt_seconds;
        if self.accumulator < FIXED_PHYSICS_DT {
            return;
        }
        let recorder = CollisionRecorder::new();
        while self.accumulator >= FIXED_PHYSICS_DT {
            self.physics_pipeline.step(
                self.gravity,
                &self.integration_parameters,
                &mut self.islands,
                &mut self.broad_phase,
                &mut self.narrow_phase,
                &mut self.bodies,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                &mut self.ccd_solver,
                &(),
                &recorder,
            );
            self.accumulator -= FIXED_PHYSICS_DT;
        }
        // Translate Rapier collision events into entity-keyed contact events.
        // Skip events whose collider handles aren't in our map (e.g., colliders
        // belonging to bodies despawned earlier this tick — Rapier may emit a
        // Stopped event for such cases).
        let raw = recorder.drain();
        self.pending_contact_events.reserve(raw.len());
        for event in raw {
            let (h1, h2, started) = match event {
                CollisionEvent::Started(a, b, _) => (a, b, true),
                CollisionEvent::Stopped(a, b, _) => (a, b, false),
            };
            let (Some(&entity_a), Some(&entity_b)) = (
                self.collider_to_entity.get(&h1),
                self.collider_to_entity.get(&h2),
            ) else {
                continue;
            };
            self.pending_contact_events.push(ContactEvent {
                entity_a,
                entity_b,
                started,
            });
        }
    }

    fn sync_outputs(&self, entities: &mut HashMap<Uuid, EntityStateEntry>, skip: &[Uuid]) {
        for (id, entry) in entities.iter_mut() {
            if skip.contains(id) {
                continue;
            }
            let Some(&handle) = self.handles.get(id) else {
                continue;
            };
            let Some(body) = self.bodies.get(handle) else {
                continue;
            };
            let t = body.translation();
            let v = body.linvel();
            entry.position.x = t.x as f64;
            entry.position.y = t.y as f64;
            entry.position.z = t.z as f64;
            entry.velocity.x = v.x as f64;
            entry.velocity.y = v.y as f64;
            entry.velocity.z = v.z as f64;
        }
    }

    fn despawn_missing(&mut self, entities: &HashMap<Uuid, EntityStateEntry>) {
        let stale: Vec<Uuid> = self
            .handles
            .keys()
            .filter(|id| !entities.contains_key(id))
            .copied()
            .collect();
        for id in stale {
            self.despawn(id);
        }
    }
}

/// User-logic backend wrapped by [`RapierClusterSim`].
enum Backend {
    /// No user-side logic — Rapier just integrates whatever `entity.velocity`
    /// is on the wire. Useful for the "background simulator" use case where
    /// clients seed velocity and the server just keeps entities moving.
    None,
    /// Standard `ClusterSimulation` user (V1 path). User mutates `entity.velocity`
    /// in their `on_tick`; default sphere collider on every spawn.
    Cluster(Arc<dyn ClusterSimulation>),
    /// Rapier-aware user (V2 path). User gets contact events and per-entity
    /// shape declarations.
    Rapier(Arc<dyn RapierClusterSimulation>),
}

/// A [`ClusterSimulation`] that runs the user's logic, then a Rapier physics step.
///
/// Three constructor flavors:
///
/// - [`RapierClusterSim::new`] — wraps an `Option<Arc<dyn ClusterSimulation>>`.
///   `None` means pure Rapier integration with no game logic. Default sphere
///   collider on every entity (radius from [`RapierConfig::default_body_radius`]).
/// - [`RapierClusterSim::with_default_config`] — same as `new` with default config.
/// - [`RapierClusterSim::with_rapier_sim`] — wraps an
///   `Arc<dyn RapierClusterSimulation>`. Unlocks per-entity collider shapes
///   (`collider_for`) and surfaces contact events to the user via
///   [`RapierClusterTickContext`].
pub struct RapierClusterSim {
    backend: Backend,
    config: RapierConfig,
    state: Mutex<RapierState>,
}

impl RapierClusterSim {
    pub fn new(user_sim: Option<Arc<dyn ClusterSimulation>>, config: RapierConfig) -> Self {
        let gravity = Vector::new(config.gravity[0], config.gravity[1], config.gravity[2]);
        let backend = match user_sim {
            Some(s) => Backend::Cluster(s),
            None => Backend::None,
        };
        Self {
            backend,
            config,
            state: Mutex::new(RapierState::new(gravity)),
        }
    }

    pub fn with_default_config(user_sim: Option<Arc<dyn ClusterSimulation>>) -> Self {
        Self::new(user_sim, RapierConfig::default())
    }

    /// V2 constructor — accepts a [`RapierClusterSimulation`]. Use this when you
    /// want per-entity collider shapes or contact events.
    pub fn with_rapier_sim(
        rapier_sim: Arc<dyn RapierClusterSimulation>,
        config: RapierConfig,
    ) -> Self {
        let gravity = Vector::new(config.gravity[0], config.gravity[1], config.gravity[2]);
        Self {
            backend: Backend::Rapier(rapier_sim),
            config,
            state: Mutex::new(RapierState::new(gravity)),
        }
    }

    fn shape_for(&self, entry: &EntityStateEntry) -> RapierColliderShape {
        match &self.backend {
            Backend::Rapier(sim) => sim.collider_for(entry, &self.config),
            _ => RapierColliderShape::Ball(self.config.default_body_radius),
        }
    }
}

impl ClusterSimulation for RapierClusterSim {
    fn on_tick(&self, ctx: &mut ClusterTickContext<'_>) {
        // Pull contact events from the previous step. The Rapier-aware backend
        // exposes them to the user via the extended context. (We take ownership
        // here rather than borrow so the user's on_tick can run without holding
        // the state lock.)
        let prev_contacts = {
            let mut state = self.state.lock().expect("rapier state lock");
            std::mem::take(&mut state.pending_contact_events)
        };

        match &self.backend {
            Backend::None => {}
            Backend::Cluster(sim) => sim.on_tick(ctx),
            Backend::Rapier(sim) => {
                let mut rapier_ctx = RapierClusterTickContext {
                    cluster_id: ctx.cluster_id,
                    tick: ctx.tick,
                    dt_seconds: ctx.dt_seconds,
                    entities: ctx.entities,
                    pending_removals: ctx.pending_removals,
                    game_actions: ctx.game_actions,
                    contact_events: &prev_contacts,
                };
                sim.on_tick(&mut rapier_ctx);
            }
        }

        let mut state = self.state.lock().expect("rapier state lock");

        // Entities the user has flagged for removal this tick. The cluster runner
        // will drop them from the entity map *after* on_tick returns, but for our
        // purposes they are already gone — no spawn, no sync, no body. Linear
        // scan over the slice; in steady state pending_removals is empty so the
        // contains() calls are cheap.
        let removed: &[Uuid] = ctx.pending_removals.as_slice();
        for &id in removed {
            state.despawn(id);
        }
        state.despawn_missing(ctx.entities);

        // Spawn new entities and sync velocity intent for existing ones.
        // Done outside RapierState so the wrapper can call self.shape_for() to
        // resolve per-entity collider shape via the active backend.
        for (id, entry) in ctx.entities.iter() {
            if removed.contains(id) {
                continue;
            }
            if state.handles.contains_key(id) {
                state.set_linvel(*id, entry.velocity.x, entry.velocity.y, entry.velocity.z);
            } else {
                let shape = self.shape_for(entry);
                state.spawn(*id, entry, shape);
            }
        }

        state.step_with_accumulator(ctx.dt_seconds as f32);
        state.sync_outputs(ctx.entities, removed);
        // Contact events from this step now live in state.pending_contact_events
        // for next tick's user_sim to read.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_core::cluster_simulation::GameAction;
    use arcane_core::Vec3;
    use std::sync::atomic::{AtomicU64, Ordering};

    const CLUSTER_DT: f64 = 1.0 / 20.0; // matches the default 20 Hz cluster tick
    const SUBSTEP_TOL: f64 = 0.05; // ~5% tolerance for substep residue

    fn mk_entry(id: Uuid, pos: Vec3, vel: Vec3) -> EntityStateEntry {
        EntityStateEntry::new(id, Uuid::nil(), pos, vel)
    }

    /// Run `sim.on_tick` once with the given dt, no actions, no removals.
    fn step_once(
        sim: &RapierClusterSim,
        entities: &mut HashMap<Uuid, EntityStateEntry>,
        tick: u64,
        dt: f64,
    ) {
        let mut pending: Vec<Uuid> = Vec::new();
        let actions: Vec<GameAction> = Vec::new();
        let mut ctx = ClusterTickContext {
            cluster_id: Uuid::nil(),
            tick,
            dt_seconds: dt,
            entities,
            pending_removals: &mut pending,
            game_actions: &actions,
        };
        sim.on_tick(&mut ctx);
    }

    fn step_n(
        sim: &RapierClusterSim,
        entities: &mut HashMap<Uuid, EntityStateEntry>,
        n: u64,
        dt: f64,
    ) {
        for tick in 0..n {
            step_once(sim, entities, tick + 1, dt);
        }
    }

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    fn handle_count(sim: &RapierClusterSim) -> usize {
        sim.state.lock().unwrap().handles.len()
    }

    // ─── lifecycle ──────────────────────────────────────────────────────────────

    #[test]
    fn empty_entities_steps_cleanly() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        step_n(&sim, &mut entities, 5, CLUSTER_DT);
        assert_eq!(handle_count(&sim), 0);
    }

    #[test]
    fn first_sight_spawn_uses_initial_position() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(5.0, 2.0, -3.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        // One short tick with zero velocity; final position should match initial.
        step_once(&sim, &mut entities, 1, CLUSTER_DT);
        let p = entities.get(&id).unwrap().position;
        assert!(close(p.x, 5.0, 1e-3), "x: {}", p.x);
        assert!(close(p.y, 2.0, 1e-3), "y: {}", p.y);
        assert!(close(p.z, -3.0, 1e-3), "z: {}", p.z);
    }

    #[test]
    fn position_writes_from_user_are_overwritten_by_rapier() {
        // Contract: AFTER first-sight spawn, user position writes are overwritten
        // by Rapier output. (At first-sight, the user-mutated position becomes the
        // spawn position — this is intentional, since the cluster runner has already
        // populated the entity map by the time on_tick runs.)
        struct PositionWriter;
        impl ClusterSimulation for PositionWriter {
            fn on_tick(&self, ctx: &mut ClusterTickContext<'_>) {
                if ctx.tick > 1 {
                    for entity in ctx.entities.values_mut() {
                        entity.position = Vec3::new(999.0, 999.0, 999.0);
                    }
                }
            }
        }
        let sim = RapierClusterSim::with_default_config(Some(Arc::new(PositionWriter)));
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        // Tick 1: writer no-op, body spawned at (0,0,0).
        // Tick 2+: writer pushes 999s, but wrapper ignores entity.position when
        // the body already exists and writes Rapier output (still 0,0,0) back.
        step_n(&sim, &mut entities, 3, CLUSTER_DT);
        let p = entities.get(&id).unwrap().position;
        assert!(p.x.abs() < 1e-3 && p.y.abs() < 1e-3 && p.z.abs() < 1e-3, "{:?}", p);
    }

    #[test]
    fn pending_removals_destroy_body() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        let id = Uuid::from_u128(7);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_once(&sim, &mut entities, 1, CLUSTER_DT);
        assert_eq!(handle_count(&sim), 1);

        // Simulate the cluster removing the entity post-tick.
        entities.remove(&id);
        step_once(&sim, &mut entities, 2, CLUSTER_DT);
        assert_eq!(handle_count(&sim), 0);
    }

    #[test]
    fn user_can_request_removal_via_pending_removals() {
        // The cluster runner consumes pending_removals AFTER on_tick returns. Inside
        // on_tick the user can push ids into pending_removals; our wrapper reads them
        // and despawns the bodies before stepping physics.
        struct RemoveAll;
        impl ClusterSimulation for RemoveAll {
            fn on_tick(&self, ctx: &mut ClusterTickContext<'_>) {
                let ids: Vec<Uuid> = ctx.entities.keys().copied().collect();
                ctx.pending_removals.extend(ids);
            }
        }
        let sim = RapierClusterSim::with_default_config(Some(Arc::new(RemoveAll)));
        let mut entities = HashMap::new();
        for k in 0..5u128 {
            let id = Uuid::from_u128(k);
            entities.insert(id, mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));
        }
        // First tick spawns 5 bodies, then the wrapper sees pending_removals with all
        // 5 ids and despawns them.
        step_once(&sim, &mut entities, 1, CLUSTER_DT);
        assert_eq!(handle_count(&sim), 0);
    }

    #[test]
    fn respawn_same_uuid_creates_fresh_body() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(42);

        // First lifetime: spawn at +10 with velocity 1, drift, despawn.
        entities.insert(
            id,
            mk_entry(id, Vec3::new(10.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 4, CLUSTER_DT); // ~0.2 s of motion
        let drifted_x = entities.get(&id).unwrap().position.x;
        assert!(drifted_x > 10.0, "expected drift, got {}", drifted_x);
        entities.remove(&id);
        step_once(&sim, &mut entities, 5, CLUSTER_DT); // despawn_missing
        assert_eq!(handle_count(&sim), 0);

        // Second lifetime: same UUID, fresh starting state.
        entities.insert(
            id,
            mk_entry(id, Vec3::new(-100.0, 5.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_once(&sim, &mut entities, 6, CLUSTER_DT);
        let p = entities.get(&id).unwrap().position;
        assert!(close(p.x, -100.0, 1e-3), "fresh body should start at -100, got {}", p.x);
        assert!(close(p.y, 5.0, 1e-3), "fresh body y, got {}", p.y);
        assert_eq!(handle_count(&sim), 1);
    }

    // ─── multi-entity ───────────────────────────────────────────────────────────

    #[test]
    fn multiple_entities_advance_independently() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities = HashMap::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        // Spaced apart (>> 0.5 default sphere radius) so contact resolution doesn't
        // perturb the linear-motion expectation.
        let a_start = Vec3::new(0.0, 0.0, 0.0);
        let b_start = Vec3::new(100.0, 0.0, 0.0);
        let c_start = Vec3::new(-100.0, 0.0, 0.0);
        entities.insert(a, mk_entry(a, a_start, Vec3::new(1.0, 0.0, 0.0)));
        entities.insert(b, mk_entry(b, b_start, Vec3::new(0.0, 2.0, 0.0)));
        entities.insert(c, mk_entry(c, c_start, Vec3::new(0.0, 0.0, -3.0)));

        step_n(&sim, &mut entities, 20, CLUSTER_DT); // 1.0 s

        let pa = entities.get(&a).unwrap().position;
        let pb = entities.get(&b).unwrap().position;
        let pc = entities.get(&c).unwrap().position;
        // Each entity should have moved by its own velocity vector × elapsed time.
        assert!(close(pa.x - a_start.x, 1.0, SUBSTEP_TOL), "Δa.x = {}", pa.x - a_start.x);
        assert!((pa.y - a_start.y).abs() < SUBSTEP_TOL);
        assert!((pa.z - a_start.z).abs() < SUBSTEP_TOL);
        assert!(close(pb.y - b_start.y, 2.0, 2.0 * SUBSTEP_TOL), "Δb.y = {}", pb.y - b_start.y);
        assert!((pb.x - b_start.x).abs() < SUBSTEP_TOL);
        assert!((pb.z - b_start.z).abs() < SUBSTEP_TOL);
        assert!(close(pc.z - c_start.z, -3.0, 3.0 * SUBSTEP_TOL), "Δc.z = {}", pc.z - c_start.z);
        assert!((pc.x - c_start.x).abs() < SUBSTEP_TOL);
        assert!((pc.y - c_start.y).abs() < SUBSTEP_TOL);
    }

    #[test]
    fn many_entities_no_crash_and_advance_proportionally() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities = HashMap::new();
        let n = 500u128;
        // Spread entities far apart so they don't overlap (default sphere radius 0.5).
        for k in 0..n {
            let id = Uuid::from_u128(k);
            let row = (k / 25) as f64;
            let col = (k % 25) as f64;
            entities.insert(
                id,
                mk_entry(id, Vec3::new(col * 5.0, 0.0, row * 5.0), Vec3::new(1.0, 0.0, 0.0)),
            );
        }
        step_n(&sim, &mut entities, 20, CLUSTER_DT); // 1.0 s
        assert_eq!(handle_count(&sim), n as usize);
        // Spot-check: every entity should have advanced by ~1.0 in x.
        for k in 0..n {
            let id = Uuid::from_u128(k);
            let entry = entities.get(&id).unwrap();
            let col = (k % 25) as f64;
            let expected = col * 5.0 + 1.0;
            assert!(
                (entry.position.x - expected).abs() < 0.1,
                "entity {} x = {}, expected ~{}",
                k,
                entry.position.x,
                expected
            );
        }
    }

    // ─── dynamics ───────────────────────────────────────────────────────────────

    #[test]
    fn velocity_passthrough_advances_position() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 60, CLUSTER_DT); // 3 s
        let final_x = entities.get(&id).unwrap().position.x;
        assert!(close(final_x, 3.0, 0.15), "expected ~3.0, got {}", final_x);
    }

    #[test]
    fn zero_velocity_zero_gravity_position_unchanged() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        let start = Vec3::new(7.0, -2.0, 11.0);
        entities.insert(id, mk_entry(id, start, Vec3::new(0.0, 0.0, 0.0)));
        step_n(&sim, &mut entities, 100, CLUSTER_DT); // 5 s
        let p = entities.get(&id).unwrap().position;
        assert!(close(p.x, start.x, 1e-3));
        assert!(close(p.y, start.y, 1e-3));
        assert!(close(p.z, start.z, 1e-3));
    }

    #[test]
    fn gravity_freefall_is_physically_plausible() {
        // Plausibility, not exact match: Rapier uses semi-implicit Euler over fixed
        // 1/60 substeps, so position over T seconds differs from analytic 0.5·g·t²
        // by O(g·dt·t). We assert the entity moves *down*, accelerates correctly
        // (velocity grows linearly), and final position is within 10% of analytic.
        let config = RapierConfig {
            gravity: [0.0, -9.81, 0.0],
            ..Default::default()
        };
        let sim = RapierClusterSim::new(None, config);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        let total_t = 1.0;
        let ticks = (total_t / CLUSTER_DT).round() as u64;
        step_n(&sim, &mut entities, ticks, CLUSTER_DT);
        let p = entities.get(&id).unwrap().position;
        let v = entities.get(&id).unwrap().velocity;
        let analytic_y = -0.5 * 9.81 * total_t * total_t;
        let analytic_vy = -9.81 * total_t;
        // Position within 10%, velocity within 5%.
        assert!(
            (p.y - analytic_y).abs() < 0.1 * analytic_y.abs(),
            "y = {}, analytic = {}",
            p.y,
            analytic_y
        );
        assert!(
            (v.y - analytic_vy).abs() < 0.05 * analytic_vy.abs(),
            "vy = {}, analytic = {}",
            v.y,
            analytic_vy
        );
    }

    #[test]
    fn velocity_change_takes_effect_on_next_step() {
        // User mutates velocity in `on_tick`; that mutation must be picked up by
        // Rapier's next step. Models the buff-multiplier pattern in BenchmarkSimulation.
        struct DoubleVx;
        impl ClusterSimulation for DoubleVx {
            fn on_tick(&self, ctx: &mut ClusterTickContext<'_>) {
                if ctx.tick == 30 {
                    for e in ctx.entities.values_mut() {
                        e.velocity.x *= 2.0;
                    }
                }
            }
        }
        let sim = RapierClusterSim::with_default_config(Some(Arc::new(DoubleVx)));
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 60, CLUSTER_DT); // 3 s; doubling kicks in at tick 30 (1.5 s mark)
        let final_x = entities.get(&id).unwrap().position.x;
        // First 1.5s at vx=1 → +1.5; remaining 1.5s at vx=2 → +3.0; total ≈ 4.5.
        assert!(close(final_x, 4.5, 0.25), "got {}", final_x);
        let final_vx = entities.get(&id).unwrap().velocity.x;
        assert!(close(final_vx, 2.0, 1e-3), "got {}", final_vx);
    }

    #[test]
    fn output_velocity_under_gravity_grows_linearly() {
        let config = RapierConfig {
            gravity: [0.0, -9.81, 0.0],
            ..Default::default()
        };
        let sim = RapierClusterSim::new(None, config);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        let mut prev_vy: Option<f64> = None;
        for tick in 0..40 {
            step_once(&sim, &mut entities, tick + 1, CLUSTER_DT);
            let vy = entities.get(&id).unwrap().velocity.y;
            if let Some(prev) = prev_vy {
                assert!(vy < prev, "vy must monotonically decrease under -y gravity (was {}, now {})", prev, vy);
            }
            prev_vy = Some(vy);
        }
    }

    // ─── user-sim composition ───────────────────────────────────────────────────

    #[test]
    fn none_user_sim_runs_pure_rapier() {
        // No wrapped user sim → Rapier still advances entities based on whatever
        // velocity is on the EntityStateEntry. Models the "low-cost background
        // simulation" use case: clients seed velocity, server just integrates.
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 20, CLUSTER_DT); // 1 s
        let p = entities.get(&id).unwrap().position;
        assert!(close(p.x, 2.0, 0.1), "x = {}", p.x);
    }

    #[test]
    fn user_on_tick_runs_before_physics_with_correct_context() {
        // The user observes the pre-physics state and gets correct dt/tick/actions.
        // After their on_tick, Rapier picks up whatever velocity they wrote.
        struct Spy {
            calls: AtomicU64,
            last_dt: Mutex<f64>,
            last_tick: AtomicU64,
            last_action_count: AtomicU64,
        }
        impl ClusterSimulation for Spy {
            fn on_tick(&self, ctx: &mut ClusterTickContext<'_>) {
                self.calls.fetch_add(1, Ordering::SeqCst);
                *self.last_dt.lock().unwrap() = ctx.dt_seconds;
                self.last_tick.store(ctx.tick, Ordering::SeqCst);
                self.last_action_count
                    .store(ctx.game_actions.len() as u64, Ordering::SeqCst);
                // Mutate velocity — Rapier should pick this up.
                for e in ctx.entities.values_mut() {
                    e.velocity = Vec3::new(5.0, 0.0, 0.0);
                }
            }
        }
        let spy = Arc::new(Spy {
            calls: AtomicU64::new(0),
            last_dt: Mutex::new(0.0),
            last_tick: AtomicU64::new(0),
            last_action_count: AtomicU64::new(0),
        });
        let sim = RapierClusterSim::with_default_config(Some(spy.clone()));
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );

        let action = GameAction {
            entity_id: id,
            action_type: "test".into(),
            payload: serde_json::Value::Null,
        };
        let actions = vec![action];
        let mut pending: Vec<Uuid> = Vec::new();
        let mut ctx = ClusterTickContext {
            cluster_id: Uuid::nil(),
            tick: 42,
            dt_seconds: CLUSTER_DT,
            entities: &mut entities,
            pending_removals: &mut pending,
            game_actions: &actions,
        };
        sim.on_tick(&mut ctx);

        assert_eq!(spy.calls.load(Ordering::SeqCst), 1);
        assert!(close(*spy.last_dt.lock().unwrap(), CLUSTER_DT, 1e-9));
        assert_eq!(spy.last_tick.load(Ordering::SeqCst), 42);
        assert_eq!(spy.last_action_count.load(Ordering::SeqCst), 1);
        // Rapier saw the velocity the spy wrote (5.0) → entity advances along x.
        let p = entities.get(&id).unwrap().position;
        assert!(p.x > 0.0, "Rapier should have applied user-written velocity, x = {}", p.x);
    }

    #[test]
    fn user_buff_modifies_velocity_then_rapier_integrates() {
        // Mimics BenchmarkSimulation's buff pattern: user multiplies velocity by a
        // speed factor each tick; Rapier integrates the buffed velocity into position.
        struct Buff(f64);
        impl ClusterSimulation for Buff {
            fn on_tick(&self, ctx: &mut ClusterTickContext<'_>) {
                for e in ctx.entities.values_mut() {
                    e.velocity.x *= self.0;
                }
            }
        }
        let sim = RapierClusterSim::with_default_config(Some(Arc::new(Buff(1.0))));
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 20, CLUSTER_DT); // 1 s of vx=1
        let baseline_x = entities.get(&id).unwrap().position.x;

        // Now redo with a 2× buff.
        let sim2 = RapierClusterSim::with_default_config(Some(Arc::new(Buff(2.0))));
        let mut entities2 = HashMap::new();
        entities2.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        );
        // With Buff(2.0), velocity doubles every tick → exponential growth. Use a
        // shorter horizon (3 ticks) so the assertion is meaningful.
        step_n(&sim2, &mut entities2, 3, CLUSTER_DT);
        let buffed_x = entities2.get(&id).unwrap().position.x;
        let buffed_vx = entities2.get(&id).unwrap().velocity.x;
        assert!(buffed_x > baseline_x / 5.0, "buff should produce more motion per tick");
        assert!(buffed_vx >= 8.0, "vx should have doubled 3× to ≥ 8, got {}", buffed_vx);
    }

    // ─── determinism / hand-off ─────────────────────────────────────────────────

    #[test]
    fn same_inputs_produce_same_outputs() {
        // Two independent RapierClusterSim instances with identical initial state
        // and identical tick sequence must produce identical final state. This is
        // the in-process determinism guarantee — important for verification and
        // server-side reconciliation.
        fn run() -> (f64, f64, f64) {
            let sim = RapierClusterSim::with_default_config(None);
            let mut entities = HashMap::new();
            let id = Uuid::from_u128(1);
            entities.insert(
                id,
                mk_entry(id, Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.5, -0.25, 0.75)),
            );
            step_n(&sim, &mut entities, 100, CLUSTER_DT);
            let p = entities.get(&id).unwrap().position;
            (p.x, p.y, p.z)
        }
        let a = run();
        let b = run();
        assert_eq!(a, b, "expected bit-identical outputs across runs");
    }

    #[test]
    fn state_round_trips_through_despawn_respawn() {
        // Models the hand-off-out / hand-off-in flow: cluster simulates entity for N
        // ticks, exports the resulting EntityStateEntry, despawns the body, then a
        // (different) cluster respawns from that exported state and continues. The
        // continuation should match running the original sim straight through for
        // N+M ticks.
        fn straight_through() -> Vec3 {
            let sim = RapierClusterSim::with_default_config(None);
            let mut entities = HashMap::new();
            let id = Uuid::from_u128(1);
            entities.insert(
                id,
                mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
            );
            step_n(&sim, &mut entities, 40, CLUSTER_DT); // 2 s
            entities.get(&id).unwrap().position
        }
        fn handoff() -> Vec3 {
            let sim_a = RapierClusterSim::with_default_config(None);
            let mut entities = HashMap::new();
            let id = Uuid::from_u128(1);
            entities.insert(
                id,
                mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
            );
            step_n(&sim_a, &mut entities, 20, CLUSTER_DT); // 1 s on cluster A
            // Hand-off: capture entry, drop sim_a, respawn on sim_b.
            let exported = entities.get(&id).unwrap().clone();
            drop(sim_a);
            let sim_b = RapierClusterSim::with_default_config(None);
            let mut entities = HashMap::new();
            entities.insert(id, exported);
            step_n(&sim_b, &mut entities, 20, CLUSTER_DT); // 1 s on cluster B
            entities.get(&id).unwrap().position
        }
        let direct = straight_through();
        let via_handoff = handoff();
        // Hand-off doesn't preserve substep accumulator residue, so a small
        // discrepancy is expected. ≤1% of a unit over 2 s of motion is acceptable.
        assert!(
            (direct.x - via_handoff.x).abs() < 0.05,
            "direct {:?} vs handoff {:?}",
            direct,
            via_handoff
        );
        assert!((direct.y - via_handoff.y).abs() < 0.05);
        assert!((direct.z - via_handoff.z).abs() < 0.05);
    }

    // ─── V2: contact events + per-entity colliders ──────────────────────────────

    /// Test helper: records every contact event the wrapper surfaces.
    struct ContactRecorder {
        events: Mutex<Vec<ContactEvent>>,
        shape: RapierColliderShape,
    }

    impl ContactRecorder {
        fn new(shape: RapierColliderShape) -> Arc<Self> {
            Arc::new(Self {
                events: Mutex::new(Vec::new()),
                shape,
            })
        }
        fn snapshot(&self) -> Vec<ContactEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl RapierClusterSimulation for ContactRecorder {
        fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
            self.events
                .lock()
                .unwrap()
                .extend_from_slice(ctx.contact_events);
        }
        fn collider_for(
            &self,
            _entry: &EntityStateEntry,
            _config: &RapierConfig,
        ) -> RapierColliderShape {
            self.shape
        }
    }

    /// Returns whether (a, b) appears in the recorded events as a Started
    /// event in either ordering.
    fn started_pair_present(events: &[ContactEvent], a: Uuid, b: Uuid) -> bool {
        events.iter().any(|e| {
            e.started
                && ((e.entity_a == a && e.entity_b == b) || (e.entity_a == b && e.entity_b == a))
        })
    }

    fn count_started_for_pair(events: &[ContactEvent], a: Uuid, b: Uuid) -> usize {
        events
            .iter()
            .filter(|e| {
                e.started
                    && ((e.entity_a == a && e.entity_b == b)
                        || (e.entity_a == b && e.entity_b == a))
            })
            .count()
    }

    #[test]
    fn contact_event_surfaces_for_overlapping_spheres() {
        let recorder = ContactRecorder::new(RapierColliderShape::Ball(0.5));
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        // Centers 0.4 apart with radius 0.5 each → significant overlap.
        entities.insert(a, mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));
        entities.insert(b, mk_entry(b, Vec3::new(0.4, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));

        // Tick 1 spawns and steps; contact emitted post-step. Tick 2 surfaces it.
        step_n(&sim, &mut entities, 2, CLUSTER_DT);

        let events = recorder.snapshot();
        assert!(
            started_pair_present(&events, a, b),
            "expected Started event for ({a}, {b}); got {events:?}"
        );
    }

    #[test]
    fn distant_capsules_produce_no_contacts() {
        let recorder = ContactRecorder::new(RapierColliderShape::Capsule {
            half_height: 1.0,
            radius: 0.4,
        });
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        // 100 units apart — well outside any collider radius.
        entities.insert(a, mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));
        entities.insert(b, mk_entry(b, Vec3::new(100.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));

        step_n(&sim, &mut entities, 5, CLUSTER_DT);

        let events = recorder.snapshot();
        assert!(events.is_empty(), "expected no contacts; got {events:?}");
    }

    #[test]
    fn collider_for_is_honored_at_first_sight() {
        // Verify the collider attached to the spawned body matches the shape
        // returned by collider_for, by directly inspecting the ColliderSet.
        let recorder = ContactRecorder::new(RapierColliderShape::Cuboid([0.7, 0.3, 0.5]));
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(id, mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));
        step_once(&sim, &mut entities, 1, CLUSTER_DT);

        let state = sim.state.lock().unwrap();
        let body_handle = *state.handles.get(&id).unwrap();
        let body = state.bodies.get(body_handle).unwrap();
        let coll_handle = body.colliders().first().copied().unwrap();
        let collider = state.colliders.get(coll_handle).unwrap();
        let cuboid = collider
            .shape()
            .as_cuboid()
            .expect("collider should be a Cuboid");
        let he = cuboid.half_extents;
        assert!((he.x - 0.7).abs() < 1e-6, "half_extents.x = {}", he.x);
        assert!((he.y - 0.3).abs() < 1e-6, "half_extents.y = {}", he.y);
        assert!((he.z - 0.5).abs() < 1e-6, "half_extents.z = {}", he.z);
    }

    #[test]
    fn shape_change_after_first_sight_is_ignored() {
        // collider_for must be called exactly once per entity (at first-sight
        // spawn); never again. Documented contract: changing the return value
        // afterwards has no effect on already-spawned bodies. We verify two
        // things: (a) collider_for gets exactly one call, and (b) the resulting
        // collider matches the first call's shape, even though later calls
        // would return a different shape.
        struct ShiftingShape {
            call_count: AtomicU64,
        }
        impl RapierClusterSimulation for ShiftingShape {
            fn on_tick(&self, _ctx: &mut RapierClusterTickContext<'_>) {}
            fn collider_for(
                &self,
                _entry: &EntityStateEntry,
                _config: &RapierConfig,
            ) -> RapierColliderShape {
                let n = self.call_count.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    RapierColliderShape::Ball(0.7)
                } else {
                    RapierColliderShape::Cuboid([1.0, 1.0, 1.0])
                }
            }
        }
        let sim_inner = Arc::new(ShiftingShape {
            call_count: AtomicU64::new(0),
        });
        let sim = RapierClusterSim::with_rapier_sim(
            sim_inner.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(id, mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));
        step_n(&sim, &mut entities, 5, CLUSTER_DT);

        assert_eq!(
            sim_inner.call_count.load(Ordering::SeqCst),
            1,
            "collider_for must be called exactly once per entity"
        );

        let state = sim.state.lock().unwrap();
        let body_handle = *state.handles.get(&id).unwrap();
        let body = state.bodies.get(body_handle).unwrap();
        let coll_handle = body.colliders().first().copied().unwrap();
        let collider = state.colliders.get(coll_handle).unwrap();
        let ball = collider
            .shape()
            .as_ball()
            .expect("collider should still be the original Ball from the first call");
        assert!((ball.radius - 0.7).abs() < 1e-6);
    }

    #[test]
    fn contact_events_one_tick_delay_to_user() {
        // Tick 1: spawn overlapping → Rapier step → contact recorded internally.
        //          User on_tick during tick 1 sees `contact_events: &[]`.
        // Tick 2: User on_tick sees the contact.
        struct PerTickSnapshot {
            per_tick: Mutex<Vec<(u64, usize)>>,
            shape: RapierColliderShape,
        }
        impl RapierClusterSimulation for PerTickSnapshot {
            fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
                self.per_tick
                    .lock()
                    .unwrap()
                    .push((ctx.tick, ctx.contact_events.len()));
            }
            fn collider_for(
                &self,
                _entry: &EntityStateEntry,
                _config: &RapierConfig,
            ) -> RapierColliderShape {
                self.shape
            }
        }
        let recorder = Arc::new(PerTickSnapshot {
            per_tick: Mutex::new(Vec::new()),
            shape: RapierColliderShape::Ball(0.5),
        });
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        entities.insert(a, mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));
        entities.insert(b, mk_entry(b, Vec3::new(0.4, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));

        step_n(&sim, &mut entities, 3, CLUSTER_DT);

        let snapshots = recorder.per_tick.lock().unwrap().clone();
        // Tick 1 must see 0 contacts (nothing has stepped yet from this sim's
        // perspective; pending_contact_events starts empty).
        assert_eq!(snapshots[0].0, 1);
        assert_eq!(snapshots[0].1, 0, "tick 1 should have no contact events yet");
        // Tick 2 must see at least one contact (the Started from tick 1's step).
        assert_eq!(snapshots[1].0, 2);
        assert!(
            snapshots[1].1 >= 1,
            "tick 2 should surface the contact from tick 1's step; got {} events",
            snapshots[1].1
        );
    }

    #[test]
    fn no_duplicate_started_event_for_persistent_overlap() {
        // Started is edge-triggered: it should fire once per contact, not every
        // tick the contact persists. Place two bodies overlapping with zero
        // velocity and zero gravity; they remain in contact for many ticks but
        // only one Started event surfaces.
        let recorder = ContactRecorder::new(RapierColliderShape::Ball(0.5));
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        entities.insert(a, mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));
        entities.insert(b, mk_entry(b, Vec3::new(0.6, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));

        step_n(&sim, &mut entities, 20, CLUSTER_DT);

        let events = recorder.snapshot();
        assert_eq!(
            count_started_for_pair(&events, a, b),
            1,
            "Started should fire exactly once for a persistent contact; got events {:?}",
            events
        );
    }
}
