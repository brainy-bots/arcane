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
//! # Per-entity spawn-time hooks
//!
//! Beyond `collider_for`, [`RapierClusterSimulation`] exposes four more hooks
//! that customize the rigid body / collider attached at first-sight spawn:
//!
//! - [`RapierClusterSimulation::body_kind_for`] — Dynamic / KinematicPositionBased
//!   / KinematicVelocityBased / Fixed. Default `Dynamic`.
//! - [`RapierClusterSimulation::material_for`] — friction / restitution / density.
//!   Default zero-friction, zero-restitution, unit-density.
//! - [`RapierClusterSimulation::collision_groups_for`] — `memberships` + `filter`
//!   bitsets following Rapier's `InteractionGroups` semantics. Default
//!   "everything collides with everything."
//! - [`RapierClusterSimulation::is_sensor`] — sensor colliders fire contact
//!   events without producing physical pushback. Default `false`.
//!
//! All five hooks (these four plus `collider_for`) are called exactly once per
//! entity, at first-sight spawn. Subsequent return-value changes are ignored
//! for already-spawned bodies — despawn and respawn to change them.
//!
//! ## Subclass-style vs property-value-style
//!
//! Per [`docs/architecture/entity-model.md`](https://github.com/brainy-bots/arcane/blob/main/docs/architecture/entity-model.md)
//! §5, two patterns are equally valid for organizing per-entity hook returns:
//!
//! - **Property-value-style** — one [`RapierClusterSimulation`] impl matches
//!   on a kind field in `entry.user_data` (or the entity's SpacetimeDB row)
//!   and returns the right body kind / shape / material / groups per entity.
//!   Cleaner for games with many or runtime-configurable kinds.
//! - **Subclass-style** — the game maintains its own per-entity routing (a
//!   `HashMap<EntityKind, Box<dyn Strategy>>` etc.) and the
//!   [`RapierClusterSimulation`] impl dispatches into it. More ergonomic for
//!   games with a small fixed catalog of entity kinds.
//!
//! Both patterns work — the hook signatures take `&EntityStateEntry` so either
//! style can read whatever the game stored to make the decision.
//!
//! **`Fixed` and clustering:** introducing `Fixed` here only changes
//! physics-side behavior (solver-skipped, only AABB tracked). Until the
//! clustering-binding epic lands, `Fixed` entities still migrate by PGP
//! affinity — they are not yet pinned to chunk ownership.
//!
//! # In-tick imperative ops
//!
//! [`RapierClusterTickContext::physics`] exposes a [`PhysicsHandle`] that lets
//! `on_tick` mutate Rapier state and run synchronous queries. All operations
//! are entity-keyed (take `Uuid`, never raw Rapier handles).
//!
//! - **Forces / impulses:** `apply_impulse`, `apply_force`, `apply_torque_impulse`.
//! - **Direct overrides:** `set_translation` (teleport), `set_linvel`, `set_angvel`.
//! - **Reads:** `linvel`, `angvel`.
//! - **Sleep control:** `wake`, `sleep`.
//! - **Spatial queries:** `raycast`, `intersections_with_shape`.
//! - **Joints:** `create_joint` (Fixed / Revolute / Spherical / Prismatic) and
//!   `remove_joint`. Joints are auto-removed when either connected entity
//!   despawns.
//!
//! Per-op contracts (Fixed-body no-op, missing-id no-panic, set_linvel ↔
//! per-tick velocity sync interaction) are documented on [`PhysicsHandle`].
//!
//! ⚠️ **Lock window.** For the Rapier-aware path (`with_rapier_sim`) the
//! cluster's state lock is held for the duration of `on_tick`. The plain
//! `ClusterSimulation` path keeps the legacy "lock released during user code"
//! behavior — it has no `PhysicsHandle` to give it.
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
//!
//! Property-value-style impl that uses every spawn-time hook:
//!
//! ```no_run
//! use arcane_core::replication_channel::EntityStateEntry;
//! use arcane_infra::{
//!     Group, RapierBodyKind, RapierClusterSimulation, RapierClusterTickContext,
//!     RapierColliderShape, RapierCollisionGroups, RapierConfig, RapierMaterial,
//! };
//!
//! struct MyGame;
//! impl RapierClusterSimulation for MyGame {
//!     fn on_tick(&self, _ctx: &mut RapierClusterTickContext<'_>) {}
//!
//!     fn body_kind_for(&self, entry: &EntityStateEntry, _c: &RapierConfig) -> RapierBodyKind {
//!         match entry.user_data.get("kind").and_then(|v| v.as_str()) {
//!             Some("wall") | Some("item") => RapierBodyKind::Fixed,
//!             Some("platform") => RapierBodyKind::KinematicPositionBased,
//!             _ => RapierBodyKind::Dynamic, // players, projectiles, etc.
//!         }
//!     }
//!
//!     fn collider_for(&self, entry: &EntityStateEntry, c: &RapierConfig) -> RapierColliderShape {
//!         match entry.user_data.get("kind").and_then(|v| v.as_str()) {
//!             Some("player") => RapierColliderShape::Capsule { half_height: 0.9, radius: 0.4 },
//!             Some("wall") => RapierColliderShape::Cuboid([5.0, 2.0, 0.5]),
//!             _ => RapierColliderShape::Ball(c.default_body_radius),
//!         }
//!     }
//!
//!     fn material_for(&self, entry: &EntityStateEntry, _c: &RapierConfig) -> RapierMaterial {
//!         match entry.user_data.get("surface").and_then(|v| v.as_str()) {
//!             Some("ice") => RapierMaterial::new(0.05, 0.0, 1.0),
//!             Some("rubber") => RapierMaterial::new(0.9, 0.8, 1.0),
//!             _ => RapierMaterial::default(),
//!         }
//!     }
//!
//!     fn collision_groups_for(
//!         &self,
//!         entry: &EntityStateEntry,
//!         _c: &RapierConfig,
//!     ) -> RapierCollisionGroups {
//!         // Projectiles don't collide with the entity that fired them, etc.
//!         match entry.user_data.get("kind").and_then(|v| v.as_str()) {
//!             Some("projectile") => RapierCollisionGroups::new(Group::GROUP_2, Group::GROUP_1),
//!             _ => RapierCollisionGroups::default(),
//!         }
//!     }
//!
//!     fn is_sensor(&self, entry: &EntityStateEntry, _c: &RapierConfig) -> bool {
//!         entry.user_data.get("kind").and_then(|v| v.as_str()) == Some("trigger_zone")
//!     }
//! }
//! ```
//!
//! In-tick imperative ops (apply impulse on collision, hitscan raycast,
//! teleport on respawn, joint creation):
//!
//! ```no_run
//! use arcane_core::Vec3;
//! use arcane_infra::{
//!     JointSpec, RapierClusterSimulation, RapierClusterTickContext, RapierColliderShape,
//! };
//! use uuid::Uuid;
//!
//! struct ActionGame {
//!     player: Uuid,
//!     barrel: Uuid,
//! }
//! impl RapierClusterSimulation for ActionGame {
//!     fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
//!         // Hitscan: raycast from player forward, knock the hit entity back.
//!         if let Some(player_pos) = ctx.entities.get(&self.player).map(|e| e.position) {
//!             let dir = Vec3::new(0.0, 0.0, 1.0);
//!             if let Some(hit) = ctx.physics.raycast(player_pos, dir, 50.0) {
//!                 ctx.physics.apply_impulse(hit.entity_id, Vec3::new(0.0, 0.0, 10.0));
//!             }
//!         }
//!
//!         // Explosion: every entity in 5m of the barrel takes a radial impulse.
//!         if let Some(barrel_pos) = ctx.entities.get(&self.barrel).map(|e| e.position) {
//!             let radius = RapierColliderShape::Ball(5.0);
//!             for hit_id in ctx.physics.intersections_with_shape(&radius, barrel_pos) {
//!                 if hit_id == self.barrel { continue; }
//!                 if let Some(p) = ctx.entities.get(&hit_id).map(|e| e.position) {
//!                     let strength = 5.0;
//!                     let away = Vec3::new(
//!                         (p.x - barrel_pos.x) * strength,
//!                         (p.y - barrel_pos.y) * strength,
//!                         (p.z - barrel_pos.z) * strength,
//!                     );
//!                     ctx.physics.apply_impulse(hit_id, away);
//!                 }
//!             }
//!         }
//!
//!         // Teleport on contact-event: when player touches a "respawn pad", relocate.
//!         for ev in ctx.contact_events.iter().filter(|e| e.started) {
//!             if ev.entity_a == self.player {
//!                 ctx.physics.set_translation(self.player, Vec3::new(0.0, 5.0, 0.0));
//!             }
//!         }
//!     }
//! }
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use rapier3d::prelude::*;
use uuid::Uuid;

use arcane_core::cluster_simulation::{ClusterSimulation, ClusterTickContext};
use arcane_core::replication_channel::EntityStateEntry;
use arcane_core::Vec3;

fn to_rapier(v: Vec3) -> Vector {
    Vector::new(v.x as f32, v.y as f32, v.z as f32)
}

fn from_rapier(v: Vector) -> Vec3 {
    Vec3::new(v.x as f64, v.y as f64, v.z as f64)
}

/// Fixed Rapier substep size. 1/60 s matches the standard physics rate.
const FIXED_PHYSICS_DT: f32 = 1.0 / 60.0;

/// Default body radius applied when no per-entity shape is declared. Override
/// per entity by implementing [`RapierClusterSimulation::collider_for`].
const DEFAULT_BODY_RADIUS: f32 = 0.5;

/// Configuration knobs for [`RapierClusterSim`].
///
/// `#[non_exhaustive]` so adding fields in future versions isn't a SemVer
/// break. Construct via `RapierConfig::default()` and the struct-update form,
/// e.g. `RapierConfig { gravity: [0.0, -9.81, 0.0], ..Default::default() }`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RapierConfig {
    /// World gravity vector in m/s². Default is zero gravity (matches benchmark
    /// parity: today's benchmark cluster does pure velocity integration with no
    /// downward acceleration). Set to e.g. `[0.0, -9.81, 0.0]` for Earth gravity
    /// along -Y.
    pub gravity: [f32; 3],
    /// Default sphere radius for entities whose collider shape isn't customized.
    /// Used by the `ClusterSimulation` constructor and as the default for
    /// [`RapierClusterSimulation::collider_for`].
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
///
/// `#[non_exhaustive]` so adding shapes (e.g. `Cylinder`, `ConvexHull`) in
/// future versions isn't a SemVer break.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
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

/// Physics body kind for an entity. Resolved at first-sight spawn via
/// [`RapierClusterSimulation::body_kind_for`]; subsequent calls are ignored
/// for already-spawned entities (despawn-and-respawn to change body kind).
///
/// See [`docs/architecture/entity-model.md`](https://github.com/brainy-bots/arcane/blob/main/docs/architecture/entity-model.md)
/// §4 for the canonical taxonomy and per-kind use cases.
///
/// **Note on `Fixed` and clustering:** introducing `Fixed` here only changes
/// physics-side behavior (solver-skipped, only AABB tracked in broadphase).
/// Until the (unfiled) clustering-binding epic lands, `Fixed` entities still
/// migrate by PGP affinity — they are not yet pinned to chunk ownership.
///
/// `#[non_exhaustive]` so adding kinds in future versions isn't a SemVer break.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum RapierBodyKind {
    /// Full physics simulation: forces, gravity, contacts all apply. Default.
    #[default]
    Dynamic,
    /// Position is controlled by game logic; physics doesn't apply forces.
    /// Used for moving platforms, elevators, custom-locomotion characters.
    KinematicPositionBased,
    /// Velocity is controlled by game logic; physics integrates that velocity
    /// into position but doesn't add forces. Mid-ground between Dynamic and
    /// KinematicPositionBased.
    KinematicVelocityBased,
    /// Solver-skipped; only AABB tracking in broadphase. Used for walls,
    /// permanent fixtures, placed structures.
    Fixed,
}

/// Per-entity physics material — friction, restitution (bounciness), density
/// (drives mass derivation from collider volume). Resolved at first-sight
/// spawn via [`RapierClusterSimulation::material_for`].
///
/// Defaults are zero-friction, zero-restitution, unit-density — matches the
/// crate's "benchmark parity" stance (no surprising deceleration / bounce
/// out of the box).
///
/// `#[non_exhaustive]` so adding fields (e.g. anisotropic friction) in future
/// versions isn't a SemVer break.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RapierMaterial {
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
}

impl RapierMaterial {
    /// Build a material from explicit friction / restitution / density values.
    pub const fn new(friction: f32, restitution: f32, density: f32) -> Self {
        Self {
            friction,
            restitution,
            density,
        }
    }
}

impl Default for RapierMaterial {
    fn default() -> Self {
        Self::new(0.0, 0.0, 1.0)
    }
}

/// Collision filtering for an entity's collider — `memberships` declares
/// which group bits this collider belongs to; `filter` declares which group
/// bits it can collide with. Two colliders generate contacts iff
/// `(a.memberships & b.filter) != 0 && (b.memberships & a.filter) != 0`,
/// matching Rapier's `InteractionGroups` semantics.
///
/// Default is "everything collides with everything" — `memberships = Group::ALL`,
/// `filter = Group::ALL`, equivalent to Rapier's `InteractionGroups::all()`.
///
/// `#[non_exhaustive]` so adding fields (e.g. solver-only flags) in future
/// versions isn't a SemVer break.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RapierCollisionGroups {
    pub memberships: Group,
    pub filter: Group,
}

impl RapierCollisionGroups {
    /// Build a groups value from explicit memberships and filter bits.
    pub const fn new(memberships: Group, filter: Group) -> Self {
        Self {
            memberships,
            filter,
        }
    }
}

impl Default for RapierCollisionGroups {
    fn default() -> Self {
        Self::new(Group::ALL, Group::ALL)
    }
}

fn build_collider(
    shape: RapierColliderShape,
    material: RapierMaterial,
    groups: RapierCollisionGroups,
    is_sensor: bool,
) -> Collider {
    let builder = match shape {
        RapierColliderShape::Ball(radius) => ColliderBuilder::ball(radius),
        RapierColliderShape::Capsule {
            half_height,
            radius,
        } => ColliderBuilder::capsule_y(half_height, radius),
        RapierColliderShape::Cuboid(he) => ColliderBuilder::cuboid(he[0], he[1], he[2]),
    };
    builder
        .friction(material.friction)
        .restitution(material.restitution)
        .density(material.density)
        .collision_groups(InteractionGroups::new(
            groups.memberships,
            groups.filter,
            InteractionTestMode::And,
        ))
        .sensor(is_sensor)
        .active_events(ActiveEvents::COLLISION_EVENTS)
        .build()
}

/// A collision detected during a Rapier step, mapped from Rapier's collider
/// handles back to entity ids. Surfaced to [`RapierClusterSimulation::on_tick`]
/// via [`RapierClusterTickContext::contact_events`].
///
/// `started == true` signals the contact started this tick; `false` signals
/// it stopped because the bodies separated. **Despawn does not surface a
/// `Stopped` event to the contact partner** — when a body is removed (via
/// `pending_removals` or by vanishing from the entity map), its contacts
/// terminate silently and the partner detects the loss by observing the
/// despawn through the entity map. (Pinned by tests.)
///
/// `#[non_exhaustive]` so adding fields (e.g. `impulse_magnitude`) in future
/// versions isn't a SemVer break.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ContactEvent {
    pub entity_a: Uuid,
    pub entity_b: Uuid,
    pub started: bool,
}

/// A hit returned by [`PhysicsHandle::raycast`]. Maps Rapier's collider hit
/// back to an entity id.
///
/// `#[non_exhaustive]` so adding fields (e.g. `feature` for sub-shape index)
/// in future versions isn't a SemVer break.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RaycastHit {
    /// The entity whose collider was hit.
    pub entity_id: Uuid,
    /// Time of impact along the ray, in units of the ray's direction length.
    /// For a unit-length direction this is the world-space distance.
    pub time_of_impact: f32,
    /// World-space hit point.
    pub point: Vec3,
    /// Surface normal at the hit point (world space).
    pub normal: Vec3,
}

impl RaycastHit {
    pub const fn new(entity_id: Uuid, time_of_impact: f32, point: Vec3, normal: Vec3) -> Self {
        Self {
            entity_id,
            time_of_impact,
            point,
            normal,
        }
    }
}

/// Joint shape for [`PhysicsHandle::create_joint`]. Anchors are in each
/// body's local space. Limits and motors are not exposed in this minimal
/// surface — add via Rapier directly if needed (`out` of scope for `#121`).
///
/// `#[non_exhaustive]` so adding variants / fields in future versions isn't a
/// SemVer break.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum JointSpec {
    /// Rigidly attaches the two bodies — no relative motion.
    Fixed {
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
    },
    /// Allows rotation around `axis` only (1 DoF rotational).
    Revolute {
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        /// Axis of rotation, expressed in body A's local frame.
        axis: Vec3,
    },
    /// Allows free rotation around the anchor (3 DoF rotational, ball-joint).
    Spherical {
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
    },
    /// Allows translation along `axis` only (1 DoF translational, slider).
    Prismatic {
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        /// Axis of translation, expressed in body A's local frame.
        axis: Vec3,
    },
}

/// Opaque handle returned by [`PhysicsHandle::create_joint`]; pass back to
/// [`PhysicsHandle::remove_joint`]. Joints are auto-removed when either of
/// the connected entities despawns — calling `remove_joint` afterwards is a
/// safe no-op (returns `false`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JointId(ImpulseJointHandle);

/// Tick context delivered to [`RapierClusterSimulation::on_tick`]. Mirrors
/// [`ClusterTickContext`] field-for-field plus Rapier-specific extensions
/// (contact events, in-tick physics handle).
///
/// `#[non_exhaustive]` so future fields aren't a SemVer break for downstream impls.
#[non_exhaustive]
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
    /// In-tick imperative physics ops — apply impulse/force/torque, teleport,
    /// raycast, intersection queries, joint creation. See [`PhysicsHandle`] for
    /// the full surface. Reads and mutations hit Rapier state synchronously
    /// while the user's `on_tick` runs.
    pub physics: PhysicsHandle<'a>,
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
    /// returns `Ball(config.default_body_radius)`. Override to vary shape per
    /// entity (read `entry.user_data` for class info, etc.). Called exactly
    /// once per entity, when its body is first created in the Rapier world.
    fn collider_for(
        &self,
        _entry: &EntityStateEntry,
        config: &RapierConfig,
    ) -> RapierColliderShape {
        RapierColliderShape::Ball(config.default_body_radius)
    }

    /// Declare the rigid-body kind (Dynamic / KinematicPositionBased /
    /// KinematicVelocityBased / Fixed) for an entity at first-sight spawn.
    /// Default returns [`RapierBodyKind::Dynamic`]. Called exactly once per
    /// entity; subsequent return-value changes are ignored for already-spawned
    /// bodies.
    fn body_kind_for(&self, _entry: &EntityStateEntry, _config: &RapierConfig) -> RapierBodyKind {
        RapierBodyKind::Dynamic
    }

    /// Declare the physics material (friction / restitution / density) for an
    /// entity at first-sight spawn. Default is zero-friction, zero-restitution,
    /// unit-density. Called exactly once per entity; subsequent return-value
    /// changes are ignored for already-spawned bodies.
    fn material_for(&self, _entry: &EntityStateEntry, _config: &RapierConfig) -> RapierMaterial {
        RapierMaterial::default()
    }

    /// Declare the collision-filter groups (memberships + filter) for an
    /// entity's collider at first-sight spawn. Default is "everything collides
    /// with everything" — `memberships = Group::ALL`, `filter = Group::ALL`.
    /// Called exactly once per entity; subsequent return-value changes are
    /// ignored for already-spawned bodies.
    fn collision_groups_for(
        &self,
        _entry: &EntityStateEntry,
        _config: &RapierConfig,
    ) -> RapierCollisionGroups {
        RapierCollisionGroups::default()
    }

    /// Declare whether the entity's collider is a sensor (fires contact events
    /// without producing physical pushback). Default is `false`. Called
    /// exactly once per entity; subsequent return-value changes are ignored
    /// for already-spawned bodies.
    fn is_sensor(&self, _entry: &EntityStateEntry, _config: &RapierConfig) -> bool {
        false
    }
}

/// Internal bundle of per-entity first-sight spawn parameters. Keeps
/// [`RapierState::spawn`]'s signature small and gives us one place to extend
/// when adding future spawn-time hooks.
struct SpawnParams {
    shape: RapierColliderShape,
    body_kind: RapierBodyKind,
    material: RapierMaterial,
    groups: RapierCollisionGroups,
    is_sensor: bool,
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
    /// Entities whose `linvel` was set imperatively this tick (via
    /// `PhysicsHandle::set_linvel` or `apply_impulse`). The per-tick
    /// `entity.velocity` → `body.linvel` sync skips these so the imperative
    /// override sticks. Cleared at the start of every tick.
    pending_imperative_linvel: HashSet<Uuid>,
}

/// Internal `EventHandler` impl that records collisions into a `Mutex<Vec>`.
/// Only `handle_collision_event` is wired up; contact-force events are out of
/// scope until impulses/forces are exposed to user code.
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
        self.events
            .into_inner()
            .expect("collision recorder mutex poisoned — a panic occurred during the physics step")
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
        // Mutex poisoning here means a panic happened mid-step inside the
        // physics pipeline; surface it via panic-on-poison rather than dropping
        // events silently.
        self.events
            .lock()
            .expect("collision recorder mutex poisoned")
            .push(event);
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
            pending_imperative_linvel: HashSet::new(),
        }
    }

    fn spawn(
        &mut self,
        entity_id: Uuid,
        entry: &EntityStateEntry,
        params: SpawnParams,
    ) -> RigidBodyHandle {
        let builder = match params.body_kind {
            RapierBodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            RapierBodyKind::KinematicPositionBased => RigidBodyBuilder::kinematic_position_based(),
            RapierBodyKind::KinematicVelocityBased => RigidBodyBuilder::kinematic_velocity_based(),
            RapierBodyKind::Fixed => RigidBodyBuilder::fixed(),
        };
        let body = builder
            .translation(to_rapier(entry.position))
            .linvel(to_rapier(entry.velocity))
            .build();
        let body_handle = self.bodies.insert(body);
        let collider_handle = self.colliders.insert_with_parent(
            build_collider(
                params.shape,
                params.material,
                params.groups,
                params.is_sensor,
            ),
            body_handle,
            &mut self.bodies,
        );
        self.handles.insert(entity_id, body_handle);
        self.collider_to_entity.insert(collider_handle, entity_id);
        body_handle
    }

    fn set_linvel(&mut self, entity_id: Uuid, vel: Vec3) {
        let Some(&handle) = self.handles.get(&entity_id) else {
            return;
        };
        if let Some(body) = self.bodies.get_mut(handle) {
            body.set_linvel(to_rapier(vel), true);
        }
    }

    fn despawn(&mut self, entity_id: Uuid) {
        let Some(body_handle) = self.handles.remove(&entity_id) else {
            return;
        };
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
        // Skip events whose collider handles aren't in our map — Rapier may
        // emit Stopped for colliders we despawned earlier this tick.
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

    fn sync_outputs(&self, entities: &mut HashMap<Uuid, EntityStateEntry>, skip: &HashSet<Uuid>) {
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
            entry.position = from_rapier(body.translation());
            entry.velocity = from_rapier(body.linvel());
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

/// In-tick imperative physics ops, exposed via [`RapierClusterTickContext::physics`].
///
/// Holds a mutable borrow of the Rapier state for the duration of the user's
/// `on_tick`; the cluster's state lock is held alongside. All operations are
/// **entity-keyed** — they take `Uuid`, never raw Rapier handles, preserving
/// Arcane's invariant that the entity map is the user's view into physics state.
///
/// **Operations on missing entity ids** return `false` / `None` without panicking.
///
/// **Operations on `Fixed` bodies** (`apply_impulse` / `apply_force` /
/// `apply_torque_impulse` / `set_linvel` / `set_angvel`) silently no-op and
/// return `false`. Gameplay code shouldn't have to query body kind before
/// applying force; the no-op is the contract.
///
/// **`set_translation` for Dynamic bodies** teleports immediately. Can violate
/// contact constraints (a body landing inside another body); Rapier resolves
/// any new contacts at the next step. Documented behavior, not a bug.
///
/// **`set_linvel` / `apply_impulse` and the per-tick velocity sync.** After
/// `on_tick`, the wrapper syncs `entity.velocity` → `body.linvel` for every
/// existing body so the declarative path keeps working. Calls that mutate
/// linvel imperatively (`set_linvel`, `apply_impulse`) mark the entity as
/// "imperatively touched" — the per-tick sync skips those entities so the
/// imperative override stays in effect for the upcoming physics step.
pub struct PhysicsHandle<'a> {
    state: &'a mut RapierState,
}

impl<'a> PhysicsHandle<'a> {
    fn new(state: &'a mut RapierState) -> Self {
        Self { state }
    }

    fn body_mut(&mut self, entity_id: Uuid) -> Option<&mut RigidBody> {
        let handle = *self.state.handles.get(&entity_id)?;
        self.state.bodies.get_mut(handle)
    }

    fn body(&self, entity_id: Uuid) -> Option<&RigidBody> {
        let handle = *self.state.handles.get(&entity_id)?;
        self.state.bodies.get(handle)
    }

    /// Apply an instantaneous linear impulse to the entity's body. Updates
    /// `body.linvel` by `impulse / mass` immediately. Marks the entity as
    /// imperatively touched so the per-tick `entity.velocity` sync doesn't
    /// clobber the change.
    ///
    /// Returns `false` if the entity has no body, or the body is `Fixed`.
    pub fn apply_impulse(&mut self, entity_id: Uuid, impulse: Vec3) -> bool {
        let Some(body) = self.body_mut(entity_id) else {
            return false;
        };
        if body.body_type() == RigidBodyType::Fixed {
            return false;
        }
        body.apply_impulse(to_rapier(impulse), true);
        self.state.pending_imperative_linvel.insert(entity_id);
        true
    }

    /// Add a continuous force, applied during the upcoming physics step.
    /// Cleared by Rapier at the start of each step.
    ///
    /// Returns `false` if the entity has no body, or the body is `Fixed`.
    pub fn apply_force(&mut self, entity_id: Uuid, force: Vec3) -> bool {
        let Some(body) = self.body_mut(entity_id) else {
            return false;
        };
        if body.body_type() == RigidBodyType::Fixed {
            return false;
        }
        body.add_force(to_rapier(force), true);
        true
    }

    /// Apply an instantaneous angular impulse (torque impulse).
    ///
    /// Returns `false` if the entity has no body, or the body is `Fixed`.
    pub fn apply_torque_impulse(&mut self, entity_id: Uuid, torque: Vec3) -> bool {
        let Some(body) = self.body_mut(entity_id) else {
            return false;
        };
        if body.body_type() == RigidBodyType::Fixed {
            return false;
        }
        body.apply_torque_impulse(to_rapier(torque), true);
        true
    }

    /// Teleport the entity's body to `position`. The new translation is
    /// reflected in `entity.position` after the tick (via `sync_outputs`).
    /// May violate contact constraints; Rapier resolves at the next step.
    ///
    /// Returns `false` if the entity has no body. Allowed on `Fixed` bodies
    /// (you can move walls), though clustering may not yet pin the new
    /// chunk ownership (see clustering-binding epic).
    pub fn set_translation(&mut self, entity_id: Uuid, position: Vec3) -> bool {
        let Some(body) = self.body_mut(entity_id) else {
            return false;
        };
        body.set_translation(to_rapier(position), true);
        true
    }

    /// Override the entity's linear velocity directly, bypassing the per-tick
    /// `entity.velocity` declarative path. Marks the entity as imperatively
    /// touched so the override sticks.
    ///
    /// Returns `false` if the entity has no body, or the body is `Fixed`.
    pub fn set_linvel(&mut self, entity_id: Uuid, linvel: Vec3) -> bool {
        let Some(body) = self.body_mut(entity_id) else {
            return false;
        };
        if body.body_type() == RigidBodyType::Fixed {
            return false;
        }
        body.set_linvel(to_rapier(linvel), true);
        self.state.pending_imperative_linvel.insert(entity_id);
        true
    }

    /// Override the entity's angular velocity directly.
    ///
    /// Returns `false` if the entity has no body, or the body is `Fixed`.
    pub fn set_angvel(&mut self, entity_id: Uuid, angvel: Vec3) -> bool {
        let Some(body) = self.body_mut(entity_id) else {
            return false;
        };
        if body.body_type() == RigidBodyType::Fixed {
            return false;
        }
        body.set_angvel(to_rapier(angvel), true);
        true
    }

    /// Read the entity's current linear velocity. Returns `None` if no body.
    pub fn linvel(&self, entity_id: Uuid) -> Option<Vec3> {
        self.body(entity_id).map(|b| from_rapier(b.linvel()))
    }

    /// Read the entity's current angular velocity. Returns `None` if no body.
    pub fn angvel(&self, entity_id: Uuid) -> Option<Vec3> {
        self.body(entity_id).map(|b| from_rapier(b.angvel()))
    }

    /// Wake a sleeping body so it rejoins simulation. Returns `false` if no body.
    pub fn wake(&mut self, entity_id: Uuid) -> bool {
        let Some(body) = self.body_mut(entity_id) else {
            return false;
        };
        body.wake_up(true);
        true
    }

    /// Force a body to sleep. Returns `false` if no body.
    pub fn sleep(&mut self, entity_id: Uuid) -> bool {
        let Some(body) = self.body_mut(entity_id) else {
            return false;
        };
        body.sleep();
        true
    }

    /// Cast a ray and return the closest entity-collider hit, if any.
    /// `direction` should be a non-zero vector — its length scales
    /// `time_of_impact` (use a unit vector if you want `time_of_impact` to
    /// equal world-space distance). Misses return `None`.
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_dist: f32) -> Option<RaycastHit> {
        let ray = Ray::new(to_rapier(origin), to_rapier(direction));
        let qp = self.state.broad_phase.as_query_pipeline(
            self.state.narrow_phase.query_dispatcher(),
            &self.state.bodies,
            &self.state.colliders,
            QueryFilter::default(),
        );
        let (handle, hit) = qp.cast_ray_and_get_normal(&ray, max_dist, true)?;
        let entity_id = *self.state.collider_to_entity.get(&handle)?;
        let hit_point = ray.origin + ray.dir * hit.time_of_impact;
        Some(RaycastHit::new(
            entity_id,
            hit.time_of_impact,
            from_rapier(hit_point),
            from_rapier(hit.normal),
        ))
    }

    /// Return the entity ids whose colliders overlap a query shape positioned
    /// at `position`. The shape is constructed transiently for the query and
    /// is not added to the world.
    pub fn intersections_with_shape(
        &self,
        shape: &RapierColliderShape,
        position: Vec3,
    ) -> Vec<Uuid> {
        let collider = build_collider(
            *shape,
            RapierMaterial::default(),
            RapierCollisionGroups::default(),
            true, // sensor=true: query only, no contact resolution
        );
        let shape_pos = Pose::from_translation(to_rapier(position));
        let qp = self.state.broad_phase.as_query_pipeline(
            self.state.narrow_phase.query_dispatcher(),
            &self.state.bodies,
            &self.state.colliders,
            QueryFilter::default(),
        );
        qp.intersect_shape(shape_pos, collider.shape())
            .filter_map(|(handle, _co)| self.state.collider_to_entity.get(&handle).copied())
            .collect()
    }

    /// Create a joint between two entities in this cluster. Returns the
    /// resulting [`JointId`], or `None` if either entity has no body.
    /// Joints are auto-removed when either entity despawns.
    pub fn create_joint(&mut self, a: Uuid, b: Uuid, joint: JointSpec) -> Option<JointId> {
        let h_a = *self.state.handles.get(&a)?;
        let h_b = *self.state.handles.get(&b)?;
        let joint_data: GenericJoint = match joint {
            JointSpec::Fixed {
                local_anchor_a,
                local_anchor_b,
            } => FixedJointBuilder::new()
                .local_anchor1(to_rapier(local_anchor_a))
                .local_anchor2(to_rapier(local_anchor_b))
                .build()
                .into(),
            JointSpec::Revolute {
                local_anchor_a,
                local_anchor_b,
                axis,
            } => RevoluteJointBuilder::new(to_rapier(axis).normalize())
                .local_anchor1(to_rapier(local_anchor_a))
                .local_anchor2(to_rapier(local_anchor_b))
                .build()
                .into(),
            JointSpec::Spherical {
                local_anchor_a,
                local_anchor_b,
            } => SphericalJointBuilder::new()
                .local_anchor1(to_rapier(local_anchor_a))
                .local_anchor2(to_rapier(local_anchor_b))
                .build()
                .into(),
            JointSpec::Prismatic {
                local_anchor_a,
                local_anchor_b,
                axis,
            } => PrismaticJointBuilder::new(to_rapier(axis).normalize())
                .local_anchor1(to_rapier(local_anchor_a))
                .local_anchor2(to_rapier(local_anchor_b))
                .build()
                .into(),
        };
        let handle = self.state.impulse_joints.insert(h_a, h_b, joint_data, true);
        Some(JointId(handle))
    }

    /// Remove a joint previously created by [`Self::create_joint`]. Returns
    /// `false` if the joint id is unknown (already removed via despawn or
    /// removed by an earlier call).
    pub fn remove_joint(&mut self, joint: JointId) -> bool {
        self.state.impulse_joints.remove(joint.0, true).is_some()
    }
}

/// User-logic backend wrapped by [`RapierClusterSim`].
enum Backend {
    /// No user-side logic — Rapier just integrates whatever `entity.velocity`
    /// is on the wire. Useful for the "background simulator" use case where
    /// clients seed velocity and the server just keeps entities moving.
    None,
    /// Plain `ClusterSimulation` — user mutates `entity.velocity` in their
    /// `on_tick`; default sphere collider on every spawn.
    Cluster(Arc<dyn ClusterSimulation>),
    /// Rapier-aware user — receives contact events via the extended context and
    /// can declare per-entity collider shapes.
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

    /// Constructor for users who want per-entity collider shapes or contact
    /// events. Accepts a [`RapierClusterSimulation`] in place of the plain
    /// `ClusterSimulation` taken by [`Self::new`].
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

    fn body_kind_for(&self, entry: &EntityStateEntry) -> RapierBodyKind {
        match &self.backend {
            Backend::Rapier(sim) => sim.body_kind_for(entry, &self.config),
            _ => RapierBodyKind::Dynamic,
        }
    }

    fn material_for(&self, entry: &EntityStateEntry) -> RapierMaterial {
        match &self.backend {
            Backend::Rapier(sim) => sim.material_for(entry, &self.config),
            _ => RapierMaterial::default(),
        }
    }

    fn collision_groups_for(&self, entry: &EntityStateEntry) -> RapierCollisionGroups {
        match &self.backend {
            Backend::Rapier(sim) => sim.collision_groups_for(entry, &self.config),
            _ => RapierCollisionGroups::default(),
        }
    }

    fn is_sensor_for(&self, entry: &EntityStateEntry) -> bool {
        match &self.backend {
            Backend::Rapier(sim) => sim.is_sensor(entry, &self.config),
            _ => false,
        }
    }
}

impl ClusterSimulation for RapierClusterSim {
    fn on_tick(&self, ctx: &mut ClusterTickContext<'_>) {
        match &self.backend {
            Backend::None => {
                // No user code; lock once and run the physics phase.
                let mut state = self.state.lock().expect("rapier state lock");
                // Discard any prior contacts — there is no listener.
                state.pending_contact_events.clear();
                state.pending_imperative_linvel.clear();
                self.run_physics_phase(&mut state, ctx);
            }
            Backend::Cluster(sim) => {
                // Plain ClusterSimulation: lock released during user code
                // (legacy behavior — plain ClusterSimulation has no PhysicsHandle).
                {
                    let mut state = self.state.lock().expect("rapier state lock");
                    // Drop prior contacts — plain ClusterSimulation can't read them.
                    state.pending_contact_events.clear();
                    state.pending_imperative_linvel.clear();
                }
                sim.on_tick(ctx);
                let mut state = self.state.lock().expect("rapier state lock");
                self.run_physics_phase(&mut state, ctx);
            }
            Backend::Rapier(sim) => {
                // Lock held through user `on_tick` so PhysicsHandle can mutate
                // Rapier state synchronously (impulses, raycasts, joints, etc.).
                let mut state = self.state.lock().expect("rapier state lock");
                let prev_contacts = std::mem::take(&mut state.pending_contact_events);
                state.pending_imperative_linvel.clear();
                {
                    let physics = PhysicsHandle::new(&mut state);
                    let mut rapier_ctx = RapierClusterTickContext {
                        cluster_id: ctx.cluster_id,
                        tick: ctx.tick,
                        dt_seconds: ctx.dt_seconds,
                        entities: ctx.entities,
                        pending_removals: ctx.pending_removals,
                        game_actions: ctx.game_actions,
                        contact_events: &prev_contacts,
                        physics,
                    };
                    sim.on_tick(&mut rapier_ctx);
                    // rapier_ctx (and physics) drop here; state is freely usable again.
                }
                self.run_physics_phase(&mut state, ctx);
            }
        }
    }
}

impl RapierClusterSim {
    /// Despawn / spawn / per-tick velocity sync / step / sync_outputs. Runs
    /// after the user's `on_tick`. Honors `state.pending_imperative_linvel`
    /// — entities whose linvel was set imperatively this tick skip the
    /// declarative `entity.velocity` → `body.linvel` sync.
    fn run_physics_phase(&self, state: &mut RapierState, ctx: &mut ClusterTickContext<'_>) {
        // The cluster runner drains pending_removals from the entity map AFTER
        // on_tick returns; from our perspective those entities are already gone
        // (no spawn, no sync, no body).
        let removed: HashSet<Uuid> = ctx.pending_removals.iter().copied().collect();
        for &id in &removed {
            state.despawn(id);
        }
        state.despawn_missing(ctx.entities);

        for (id, entry) in ctx.entities.iter() {
            if removed.contains(id) {
                continue;
            }
            if state.handles.contains_key(id) {
                // Skip the declarative linvel sync for entities whose linvel
                // was set imperatively this tick (via PhysicsHandle::set_linvel
                // or apply_impulse) — the imperative override wins.
                if !state.pending_imperative_linvel.contains(id) {
                    state.set_linvel(*id, entry.velocity);
                }
            } else {
                let params = SpawnParams {
                    shape: self.shape_for(entry),
                    body_kind: self.body_kind_for(entry),
                    material: self.material_for(entry),
                    groups: self.collision_groups_for(entry),
                    is_sensor: self.is_sensor_for(entry),
                };
                state.spawn(*id, entry, params);
            }
        }

        state.step_with_accumulator(ctx.dt_seconds as f32);
        state.sync_outputs(ctx.entities, &removed);
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
        let neighbors = HashMap::new();
        let mut ctx = ClusterTickContext {
            cluster_id: Uuid::nil(),
            tick,
            dt_seconds: dt,
            entities,
            pending_removals: &mut pending,
            game_actions: &actions,
            neighbor_entities: &neighbors,
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
        assert!(
            p.x.abs() < 1e-3 && p.y.abs() < 1e-3 && p.z.abs() < 1e-3,
            "{:?}",
            p
        );
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
            entities.insert(
                id,
                mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            );
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
        assert!(
            close(p.x, -100.0, 1e-3),
            "fresh body should start at -100, got {}",
            p.x
        );
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
        assert!(
            close(pa.x - a_start.x, 1.0, SUBSTEP_TOL),
            "Δa.x = {}",
            pa.x - a_start.x
        );
        assert!((pa.y - a_start.y).abs() < SUBSTEP_TOL);
        assert!((pa.z - a_start.z).abs() < SUBSTEP_TOL);
        assert!(
            close(pb.y - b_start.y, 2.0, 2.0 * SUBSTEP_TOL),
            "Δb.y = {}",
            pb.y - b_start.y
        );
        assert!((pb.x - b_start.x).abs() < SUBSTEP_TOL);
        assert!((pb.z - b_start.z).abs() < SUBSTEP_TOL);
        assert!(
            close(pc.z - c_start.z, -3.0, 3.0 * SUBSTEP_TOL),
            "Δc.z = {}",
            pc.z - c_start.z
        );
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
                mk_entry(
                    id,
                    Vec3::new(col * 5.0, 0.0, row * 5.0),
                    Vec3::new(1.0, 0.0, 0.0),
                ),
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
                assert!(
                    vy < prev,
                    "vy must monotonically decrease under -y gravity (was {}, now {})",
                    prev,
                    vy
                );
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
        let neighbors = HashMap::new();
        let mut ctx = ClusterTickContext {
            cluster_id: Uuid::nil(),
            tick: 42,
            dt_seconds: CLUSTER_DT,
            entities: &mut entities,
            pending_removals: &mut pending,
            game_actions: &actions,
            neighbor_entities: &neighbors,
        };
        sim.on_tick(&mut ctx);

        assert_eq!(spy.calls.load(Ordering::SeqCst), 1);
        assert!(close(*spy.last_dt.lock().unwrap(), CLUSTER_DT, 1e-9));
        assert_eq!(spy.last_tick.load(Ordering::SeqCst), 42);
        assert_eq!(spy.last_action_count.load(Ordering::SeqCst), 1);
        // Rapier saw the velocity the spy wrote (5.0) → entity advances along x.
        let p = entities.get(&id).unwrap().position;
        assert!(
            p.x > 0.0,
            "Rapier should have applied user-written velocity, x = {}",
            p.x
        );
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
        assert!(
            buffed_x > baseline_x / 5.0,
            "buff should produce more motion per tick"
        );
        assert!(
            buffed_vx >= 8.0,
            "vx should have doubled 3× to ≥ 8, got {}",
            buffed_vx
        );
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

    // ─── contact events + per-entity colliders ──────────────────────────────────

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
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(0.4, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );

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
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(100.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );

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
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
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
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
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
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(0.4, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );

        step_n(&sim, &mut entities, 3, CLUSTER_DT);

        let snapshots = recorder.per_tick.lock().unwrap().clone();
        // Tick 1 must see 0 contacts (nothing has stepped yet from this sim's
        // perspective; pending_contact_events starts empty).
        assert_eq!(snapshots[0].0, 1);
        assert_eq!(
            snapshots[0].1, 0,
            "tick 1 should have no contact events yet"
        );
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
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(0.6, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );

        step_n(&sim, &mut entities, 20, CLUSTER_DT);

        let events = recorder.snapshot();
        assert_eq!(
            count_started_for_pair(&events, a, b),
            1,
            "Started should fire exactly once for a persistent contact; got events {:?}",
            events
        );
    }

    // ─── extended V2 coverage: contract pinning + symmetry with V1 ──────────────

    fn stopped_pair_present(events: &[ContactEvent], a: Uuid, b: Uuid) -> bool {
        events.iter().any(|e| {
            !e.started
                && ((e.entity_a == a && e.entity_b == b) || (e.entity_a == b && e.entity_b == a))
        })
    }

    /// Direct collider-shape inspection helper. Returns the rapier collider
    /// attached to the body for this entity, or None if not spawned yet.
    fn with_collider<R>(
        sim: &RapierClusterSim,
        id: Uuid,
        f: impl FnOnce(&Collider) -> R,
    ) -> Option<R> {
        let state = sim.state.lock().unwrap();
        let body_handle = *state.handles.get(&id)?;
        let body = state.bodies.get(body_handle)?;
        let coll_handle = body.colliders().first().copied()?;
        let coll = state.colliders.get(coll_handle)?;
        Some(f(coll))
    }

    /// **T1**: a contact that *ends* (bodies move apart) surfaces a Stopped
    /// event in the next tick's contact_events. Without this, gameplay code
    /// that relies on "exited zone" / "broke contact" signals silently breaks.
    #[test]
    fn stopped_event_surfaces_when_bodies_separate() {
        let recorder = ContactRecorder::new(RapierColliderShape::Ball(0.4));
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        // Start overlapping so Started fires immediately.
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(0.6, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 2, CLUSTER_DT);
        assert!(
            started_pair_present(&recorder.snapshot(), a, b),
            "Started must surface before we test Stopped"
        );

        // Now drive B away fast enough that contact resolves to "stopped".
        // 5 units/sec for 1 second → covers >> 2 × radius gap.
        entities.get_mut(&b).unwrap().velocity = Vec3::new(5.0, 0.0, 0.0);
        step_n(&sim, &mut entities, 30, CLUSTER_DT);

        let events = recorder.snapshot();
        assert!(
            stopped_pair_present(&events, a, b),
            "Stopped must surface when bodies separate; events were {:?}",
            events
        );
    }

    /// **T2**: when a body is despawned mid-contact, the contact partner does
    /// **NOT** receive a Stopped event in the next tick. The contact
    /// terminates silently because the despawned collider is dropped from the
    /// reverse map before the post-step event drain. This is documented
    /// behavior — partners detect the loss by observing the despawn through
    /// the entity map (`pending_removals` or vanishing from `ctx.entities`).
    #[test]
    fn despawn_during_contact_does_not_surface_stopped_event() {
        struct DespawnAOnTick3 {
            events: Mutex<Vec<ContactEvent>>,
            a_id: Uuid,
        }
        impl RapierClusterSimulation for DespawnAOnTick3 {
            fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
                self.events
                    .lock()
                    .unwrap()
                    .extend_from_slice(ctx.contact_events);
                if ctx.tick == 3 {
                    ctx.pending_removals.push(self.a_id);
                }
            }
            fn collider_for(
                &self,
                _entry: &EntityStateEntry,
                _config: &RapierConfig,
            ) -> RapierColliderShape {
                RapierColliderShape::Ball(0.4)
            }
        }
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let recorder = Arc::new(DespawnAOnTick3 {
            events: Mutex::new(Vec::new()),
            a_id: a,
        });
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(0.5, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );

        step_n(&sim, &mut entities, 6, CLUSTER_DT);

        let events = recorder.events.lock().unwrap().clone();
        assert!(
            started_pair_present(&events, a, b),
            "Started should have fired before the despawn"
        );
        assert!(
            !stopped_pair_present(&events, a, b),
            "Despawn must NOT surface a Stopped event; events were {:?}",
            events
        );
    }

    /// **T3**: V1 default path produces an actual sphere collider with the
    /// configured radius. Catches any regression where the default builder
    /// silently swaps shape (would pass dynamics tests since pose round-trips
    /// either way).
    #[test]
    fn default_path_collider_is_a_ball_with_config_radius() {
        let config = RapierConfig {
            default_body_radius: 0.42,
            ..Default::default()
        };
        let sim = RapierClusterSim::new(None, config);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_once(&sim, &mut entities, 1, CLUSTER_DT);

        let radius = with_collider(&sim, id, |c| c.shape().as_ball().map(|b| b.radius))
            .flatten()
            .expect("collider should be a Ball");
        assert!((radius - 0.42).abs() < 1e-6, "ball radius = {}", radius);
    }

    /// **T4**: capsule shape declared via `collider_for` produces an actual
    /// capsule collider in Rapier — same direct-inspection invariant as T3,
    /// but for the V2 path.
    #[test]
    fn capsule_collider_is_honored_at_first_sight() {
        let recorder = ContactRecorder::new(RapierColliderShape::Capsule {
            half_height: 0.9,
            radius: 0.4,
        });
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_once(&sim, &mut entities, 1, CLUSTER_DT);

        let capsule_radius =
            with_collider(&sim, id, |c| c.shape().as_capsule().map(|cap| cap.radius))
                .flatten()
                .expect("collider should be a Capsule");
        assert!((capsule_radius - 0.4).abs() < 1e-6);
    }

    /// **T5**: a single cluster tick whose `dt_seconds` exceeds `FIXED_PHYSICS_DT`
    /// should run multiple Rapier substeps in one call to `on_tick`. With
    /// `dt = 0.1 s` and `FIXED_PHYSICS_DT = 1/60 s`, the accumulator drains
    /// 6 substeps; an entity at `vx = 1.0` should advance ≈ 0.1 m, not just
    /// `1/60` m. Catches accumulator-truncation bugs that would silently
    /// compress motion under slow cluster ticks.
    #[test]
    fn multi_substep_in_one_cluster_tick() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        );
        step_once(&sim, &mut entities, 1, 0.1); // single tick, 100 ms wall-time
        let x = entities.get(&id).unwrap().position.x;
        // Six substeps × (1/60 s) × 1 m/s = 0.1 m exactly. Allow a tiny epsilon.
        assert!(
            x > 0.09 && x < 0.11,
            "expected ≈0.1 from 6 substeps, got {}",
            x
        );
    }

    /// **T6**: when each cluster tick's `dt_seconds < FIXED_PHYSICS_DT`, the
    /// accumulator grows over multiple ticks before a substep finally fires.
    /// Catches a regression where a fast cluster (e.g., 200 Hz) never
    /// advances physics because the accumulator path is broken.
    #[test]
    fn slow_dt_accumulates_until_substep_fires() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        );

        // First tick: dt = 0.005 < FIXED_PHYSICS_DT (0.0167). Accumulator
        // shouldn't have drained, so position should still be 0.
        step_once(&sim, &mut entities, 1, 0.005);
        let after_one = entities.get(&id).unwrap().position.x;
        assert!(
            after_one.abs() < 1e-6,
            "first sub-substep tick should not advance position; got {}",
            after_one
        );

        // Continue. Over 30 ticks at 0.005 s = 0.15 s total, ≥ 8 substeps fire.
        step_n(&sim, &mut entities, 30, 0.005);
        let after_many = entities.get(&id).unwrap().position.x;
        // Total motion is roughly total_dt × velocity, minus at most one substep
        // worth of leftover accumulator.
        assert!(
            after_many > 0.10 && after_many < 0.16,
            "expected ≈0.15 ± one substep, got {}",
            after_many
        );
    }

    /// **T7**: contact resolution actually applies impulse — Rapier doesn't
    /// just *detect* collisions, it responds to them. Without this, a config
    /// that accidentally turned all colliders into sensors (no force exchange)
    /// would still pass every other contact test.
    #[test]
    fn contact_resolution_applies_impulse_to_partner() {
        let sim = RapierClusterSim::with_default_config(None);
        let mut entities = HashMap::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        // A heads at B (stationary). After collision, B must have non-zero
        // velocity in +x (got pushed) — that's contact response in action.
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 40, CLUSTER_DT); // 2 s — plenty for collision + post-collision

        let b_vel_x = entities.get(&b).unwrap().velocity.x;
        let b_pos_x = entities.get(&b).unwrap().position.x;
        assert!(
            b_vel_x > 1e-3,
            "B should have been pushed by contact resolution; vx = {}",
            b_vel_x
        );
        assert!(
            b_pos_x > 2.0,
            "B should have moved in +x from contact; pos.x = {}",
            b_pos_x
        );
    }

    /// **T8**: respawning an entity with the same UUID is a *new* first-sight,
    /// so `collider_for` should be invoked again. Important for respawn
    /// mechanics where a dead entity comes back as a different shape (e.g.,
    /// ghost form).
    #[test]
    fn collider_for_invoked_freshly_on_respawn() {
        struct CountedShape {
            calls: AtomicU64,
        }
        impl RapierClusterSimulation for CountedShape {
            fn on_tick(&self, _ctx: &mut RapierClusterTickContext<'_>) {}
            fn collider_for(
                &self,
                _entry: &EntityStateEntry,
                _config: &RapierConfig,
            ) -> RapierColliderShape {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    RapierColliderShape::Ball(0.5)
                } else {
                    RapierColliderShape::Cuboid([0.3, 0.3, 0.3])
                }
            }
        }
        let inner = Arc::new(CountedShape {
            calls: AtomicU64::new(0),
        });
        let sim = RapierClusterSim::with_rapier_sim(
            inner.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(99);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_once(&sim, &mut entities, 1, CLUSTER_DT);
        // First lifetime: Ball.
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
        assert!(with_collider(&sim, id, |c| c.shape().as_ball().is_some()).unwrap_or(false));

        // Despawn (vanish from map), let despawn_missing fire.
        entities.remove(&id);
        step_once(&sim, &mut entities, 2, CLUSTER_DT);

        // Respawn same UUID → fresh first-sight → collider_for called again.
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_once(&sim, &mut entities, 3, CLUSTER_DT);
        assert_eq!(
            inner.calls.load(Ordering::SeqCst),
            2,
            "collider_for must be called again on respawn"
        );
        assert!(
            with_collider(&sim, id, |c| c.shape().as_cuboid().is_some()).unwrap_or(false),
            "respawned body should use the second-call shape"
        );
    }

    /// **T9**: V2 user receives `game_actions` correctly through the extended
    /// context. Symmetry with the V1 `user_on_tick_runs_before_physics_with_correct_context`
    /// test for `ClusterTickContext`.
    #[test]
    fn rapier_ctx_propagates_game_actions_tick_and_dt() {
        struct Spy {
            seen_tick: AtomicU64,
            seen_dt: Mutex<f64>,
            seen_action_count: AtomicU64,
        }
        impl RapierClusterSimulation for Spy {
            fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
                self.seen_tick.store(ctx.tick, Ordering::SeqCst);
                *self.seen_dt.lock().unwrap() = ctx.dt_seconds;
                self.seen_action_count
                    .store(ctx.game_actions.len() as u64, Ordering::SeqCst);
            }
        }
        let spy = Arc::new(Spy {
            seen_tick: AtomicU64::new(0),
            seen_dt: Mutex::new(0.0),
            seen_action_count: AtomicU64::new(0),
        });
        let sim = RapierClusterSim::with_rapier_sim(
            spy.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );

        let actions = vec![
            GameAction {
                entity_id: id,
                action_type: "use_item".into(),
                payload: serde_json::Value::Null,
            },
            GameAction {
                entity_id: id,
                action_type: "interact".into(),
                payload: serde_json::Value::Null,
            },
        ];
        let mut pending: Vec<Uuid> = Vec::new();
        let neighbors = HashMap::new();
        let mut ctx = ClusterTickContext {
            cluster_id: Uuid::nil(),
            tick: 99,
            dt_seconds: CLUSTER_DT,
            entities: &mut entities,
            pending_removals: &mut pending,
            game_actions: &actions,
            neighbor_entities: &neighbors,
        };
        sim.on_tick(&mut ctx);

        assert_eq!(spy.seen_tick.load(Ordering::SeqCst), 99);
        assert!(close(*spy.seen_dt.lock().unwrap(), CLUSTER_DT, 1e-9));
        assert_eq!(spy.seen_action_count.load(Ordering::SeqCst), 2);
    }

    /// **T10**: V2 user can request entity removal via `pending_removals` —
    /// the wrapper despawns those bodies in the same tick. Symmetry with the
    /// V1 `user_can_request_removal_via_pending_removals` test.
    #[test]
    fn rapier_user_can_request_removal_via_pending_removals() {
        struct DropAll;
        impl RapierClusterSimulation for DropAll {
            fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
                let ids: Vec<Uuid> = ctx.entities.keys().copied().collect();
                ctx.pending_removals.extend(ids);
            }
        }
        let sim = RapierClusterSim::with_rapier_sim(
            Arc::new(DropAll) as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        for k in 0..4u128 {
            let id = Uuid::from_u128(k);
            entities.insert(
                id,
                mk_entry(id, Vec3::new(k as f64, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            );
        }
        step_once(&sim, &mut entities, 1, CLUSTER_DT);
        assert_eq!(handle_count(&sim), 0);
    }

    /// **T11**: a Ball and a Cuboid colliding produce a contact event. Cross-
    /// shape collision is exercised by Rapier's narrow phase but never tested
    /// elsewhere in this suite (all V2 tests pair same-shape bodies).
    #[test]
    fn mixed_shape_ball_vs_cuboid_produces_contact() {
        let ball_id = Uuid::from_u128(1);
        let box_id = Uuid::from_u128(2);
        struct MixedShape {
            ball_id: Uuid,
            events: Mutex<Vec<ContactEvent>>,
        }
        impl RapierClusterSimulation for MixedShape {
            fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
                self.events
                    .lock()
                    .unwrap()
                    .extend_from_slice(ctx.contact_events);
            }
            fn collider_for(
                &self,
                entry: &EntityStateEntry,
                _config: &RapierConfig,
            ) -> RapierColliderShape {
                if entry.entity_id == self.ball_id {
                    RapierColliderShape::Ball(0.5)
                } else {
                    RapierColliderShape::Cuboid([0.5, 0.5, 0.5])
                }
            }
        }
        let recorder = Arc::new(MixedShape {
            ball_id,
            events: Mutex::new(Vec::new()),
        });
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        // Ball (radius 0.5) at origin; cuboid (half-extents 0.5) at (0.7, 0, 0)
        // → cuboid spans x ∈ [0.2, 1.2]; sphere extends to x = 0.5. Overlap.
        entities.insert(
            ball_id,
            mk_entry(ball_id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            box_id,
            mk_entry(box_id, Vec3::new(0.7, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 2, CLUSTER_DT);

        let events = recorder.events.lock().unwrap().clone();
        assert!(
            started_pair_present(&events, ball_id, box_id),
            "ball-vs-cuboid Started event missing; events were {:?}",
            events
        );
    }

    /// **T12**: gravity is honored on any axis, not just `-Y`. A horizontal
    /// gravity vector should accelerate a stationary entity in the gravity
    /// direction. Catches a regression where gravity is hardcoded to a
    /// single axis somewhere.
    #[test]
    fn nondefault_gravity_honored_on_arbitrary_axis() {
        let config = RapierConfig {
            gravity: [3.0, 0.0, 0.0],
            ..Default::default()
        };
        let sim = RapierClusterSim::new(None, config);
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 20, CLUSTER_DT); // 1.0 s

        let p = entities.get(&id).unwrap().position;
        let v = entities.get(&id).unwrap().velocity;
        // Free-fall along +X: pos ≈ 0.5·g·t² ≈ 1.5; vx ≈ g·t = 3.
        // Wide tolerance for semi-implicit Euler at 1/60 substeps.
        assert!(
            p.x > 1.3,
            "x should accelerate in +x under +x gravity; got {}",
            p.x
        );
        assert!(v.x > 2.7, "vx should grow under +x gravity; got {}", v.x);
        // No motion on other axes.
        assert!(p.y.abs() < 1e-3 && p.z.abs() < 1e-3);
        assert!(v.y.abs() < 1e-3 && v.z.abs() < 1e-3);
    }

    /// **T13**: when an entity hands off from one cluster to another (via
    /// despawn-and-respawn on the new cluster), the receiving cluster's
    /// `contact_events` start empty — contacts don't carry across the
    /// hand-off. Documented contract; pin it explicitly.
    #[test]
    fn contact_events_do_not_carry_across_handoff() {
        let recorder_a = ContactRecorder::new(RapierColliderShape::Ball(0.4));
        let sim_a = RapierClusterSim::with_rapier_sim(
            recorder_a.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(0.5, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_n(&sim_a, &mut entities, 3, CLUSTER_DT);
        let events_a = recorder_a.snapshot();
        assert!(
            !events_a.is_empty(),
            "cluster A should have observed contacts before handoff"
        );

        // Hand off: drop sim_a, build sim_b with same entity state but a new
        // recorder. Sim_b's first on_tick must see contact_events == &[].
        drop(sim_a);
        let recorder_b = ContactRecorder::new(RapierColliderShape::Ball(0.4));
        let sim_b = RapierClusterSim::with_rapier_sim(
            recorder_b.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );

        let actions: Vec<GameAction> = Vec::new();
        let mut pending: Vec<Uuid> = Vec::new();
        let neighbors = HashMap::new();
        let mut ctx = ClusterTickContext {
            cluster_id: Uuid::nil(),
            tick: 1,
            dt_seconds: CLUSTER_DT,
            entities: &mut entities,
            pending_removals: &mut pending,
            game_actions: &actions,
            neighbor_entities: &neighbors,
        };
        sim_b.on_tick(&mut ctx);

        // Cluster B's recorder should not have seen any of cluster A's events.
        let events_b_first_tick = recorder_b.snapshot();
        assert!(
            events_b_first_tick.is_empty(),
            "cluster B's first tick must not inherit cluster A's contacts; got {:?}",
            events_b_first_tick
        );
    }

    /// **T14**: `RapierColliderShape::Capsule` is built along the **Y** axis.
    /// Verifies the documented orientation by inspecting the resulting shape's
    /// segment endpoints.
    #[test]
    fn capsule_axis_is_y() {
        let recorder = ContactRecorder::new(RapierColliderShape::Capsule {
            half_height: 1.5,
            radius: 0.3,
        });
        let sim = RapierClusterSim::with_rapier_sim(
            recorder.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let id = Uuid::from_u128(1);
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_once(&sim, &mut entities, 1, CLUSTER_DT);

        let segment = with_collider(&sim, id, |c| c.shape().as_capsule().map(|cap| cap.segment))
            .flatten()
            .expect("collider should be a Capsule");
        // capsule_y: endpoints at (0, ±half_height, 0).
        assert!((segment.a.x).abs() < 1e-6);
        assert!((segment.a.z).abs() < 1e-6);
        assert!((segment.b.x).abs() < 1e-6);
        assert!((segment.b.z).abs() < 1e-6);
        let y_extent = (segment.b.y - segment.a.y).abs();
        assert!(
            (y_extent - 3.0).abs() < 1e-5,
            "expected segment along Y of length 2·half_height = 3.0; got {}",
            y_extent
        );
    }

    // ─── per-entity hooks (#120): body kind / material / groups / sensor ───────

    /// Per-entity overrides for the `HookSim` test fixture below. `None` means
    /// "use the trait default for this hook on this entity."
    #[derive(Clone, Default)]
    struct EntitySpec {
        shape: Option<RapierColliderShape>,
        body_kind: Option<RapierBodyKind>,
        material: Option<RapierMaterial>,
        groups: Option<RapierCollisionGroups>,
        is_sensor: Option<bool>,
    }

    /// Generic test fixture exercising all five spawn-time hooks. Records
    /// per-(hook, entity) call counts so tests can assert exact-once invariants
    /// directly. Records contact events from the previous tick.
    struct HookSim {
        per_entity: Mutex<HashMap<Uuid, EntitySpec>>,
        contact_events: Mutex<Vec<ContactEvent>>,
        counts: Mutex<HashMap<(&'static str, Uuid), u64>>,
    }

    impl HookSim {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                per_entity: Mutex::new(HashMap::new()),
                contact_events: Mutex::new(Vec::new()),
                counts: Mutex::new(HashMap::new()),
            })
        }

        fn set(&self, id: Uuid, spec: EntitySpec) {
            self.per_entity.lock().unwrap().insert(id, spec);
        }

        fn count(&self, hook: &'static str, id: Uuid) -> u64 {
            *self.counts.lock().unwrap().get(&(hook, id)).unwrap_or(&0)
        }

        fn snapshot_events(&self) -> Vec<ContactEvent> {
            self.contact_events.lock().unwrap().clone()
        }

        fn bump(&self, hook: &'static str, id: Uuid) {
            *self.counts.lock().unwrap().entry((hook, id)).or_insert(0) += 1;
        }

        fn spec_for(&self, id: Uuid) -> EntitySpec {
            self.per_entity
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .unwrap_or_default()
        }
    }

    impl RapierClusterSimulation for HookSim {
        fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
            self.contact_events
                .lock()
                .unwrap()
                .extend_from_slice(ctx.contact_events);
        }

        fn collider_for(
            &self,
            entry: &EntityStateEntry,
            config: &RapierConfig,
        ) -> RapierColliderShape {
            self.bump("collider_for", entry.entity_id);
            self.spec_for(entry.entity_id)
                .shape
                .unwrap_or(RapierColliderShape::Ball(config.default_body_radius))
        }

        fn body_kind_for(&self, entry: &EntityStateEntry, _: &RapierConfig) -> RapierBodyKind {
            self.bump("body_kind_for", entry.entity_id);
            self.spec_for(entry.entity_id).body_kind.unwrap_or_default()
        }

        fn material_for(&self, entry: &EntityStateEntry, _: &RapierConfig) -> RapierMaterial {
            self.bump("material_for", entry.entity_id);
            self.spec_for(entry.entity_id).material.unwrap_or_default()
        }

        fn collision_groups_for(
            &self,
            entry: &EntityStateEntry,
            _: &RapierConfig,
        ) -> RapierCollisionGroups {
            self.bump("collision_groups_for", entry.entity_id);
            self.spec_for(entry.entity_id).groups.unwrap_or_default()
        }

        fn is_sensor(&self, entry: &EntityStateEntry, _: &RapierConfig) -> bool {
            self.bump("is_sensor", entry.entity_id);
            self.spec_for(entry.entity_id).is_sensor.unwrap_or(false)
        }
    }

    /// **#120-T1**: a `Fixed` body must not move under gravity. Verifies
    /// `body_kind_for` is honored: solver skips the body, position stays put.
    #[test]
    fn fixed_body_does_not_move_under_gravity() {
        let sim_arc = HookSim::new();
        let id = Uuid::from_u128(1);
        sim_arc.set(
            id,
            EntitySpec {
                body_kind: Some(RapierBodyKind::Fixed),
                ..Default::default()
            },
        );
        let config = RapierConfig {
            gravity: [0.0, -9.81, 0.0],
            ..Default::default()
        };
        let sim = RapierClusterSim::with_rapier_sim(
            sim_arc.clone() as Arc<dyn RapierClusterSimulation>,
            config,
        );

        let start = Vec3::new(0.0, 5.0, 0.0);
        let mut entities = HashMap::new();
        entities.insert(id, mk_entry(id, start, Vec3::new(0.0, 0.0, 0.0)));

        // 2 seconds under -9.81 — a Dynamic body would be at ~y = -14.6.
        step_n(&sim, &mut entities, 40, CLUSTER_DT);

        let p = entities.get(&id).unwrap().position;
        assert!(
            close(p.y, start.y, 1e-6),
            "Fixed body moved under gravity: y = {} (expected {})",
            p.y,
            start.y
        );
    }

    /// **#120-T2**: a `KinematicPositionBased` body ignores forces. Like Fixed
    /// it doesn't fall under gravity, but unlike Fixed its position is meant
    /// to be game-controlled (Rapier just doesn't apply forces to it).
    #[test]
    fn kinematic_position_based_ignores_gravity() {
        let sim_arc = HookSim::new();
        let id = Uuid::from_u128(1);
        sim_arc.set(
            id,
            EntitySpec {
                body_kind: Some(RapierBodyKind::KinematicPositionBased),
                ..Default::default()
            },
        );
        let config = RapierConfig {
            gravity: [0.0, -9.81, 0.0],
            ..Default::default()
        };
        let sim = RapierClusterSim::with_rapier_sim(
            sim_arc.clone() as Arc<dyn RapierClusterSimulation>,
            config,
        );

        let start = Vec3::new(0.0, 5.0, 0.0);
        let mut entities = HashMap::new();
        entities.insert(id, mk_entry(id, start, Vec3::new(0.0, 0.0, 0.0)));

        step_n(&sim, &mut entities, 40, CLUSTER_DT);

        let p = entities.get(&id).unwrap().position;
        assert!(
            close(p.y, start.y, 1e-3),
            "KinematicPositionBased body fell under gravity: y = {}",
            p.y
        );
    }

    /// **#120-T3**: `material_for` is honored — friction / restitution / density
    /// land on the resulting collider. Structural test (direct collider read);
    /// the dynamic effect is covered by the bounce test below.
    #[test]
    fn material_for_is_honored_on_collider() {
        let sim_arc = HookSim::new();
        let id = Uuid::from_u128(1);
        sim_arc.set(
            id,
            EntitySpec {
                material: Some(RapierMaterial::new(0.42, 0.73, 5.5)),
                ..Default::default()
            },
        );
        let sim = RapierClusterSim::with_rapier_sim(
            sim_arc.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );

        let mut entities = HashMap::new();
        entities.insert(
            id,
            mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_once(&sim, &mut entities, 1, CLUSTER_DT);

        let (friction, restitution, density) =
            with_collider(&sim, id, |c| (c.friction(), c.restitution(), c.density()))
                .expect("collider exists");
        assert!((friction - 0.42).abs() < 1e-5, "friction = {}", friction);
        assert!(
            (restitution - 0.73).abs() < 1e-5,
            "restitution = {}",
            restitution
        );
        assert!((density - 5.5).abs() < 1e-5, "density = {}", density);
    }

    /// **#120-T4**: high restitution produces a noticeably bouncier collision
    /// than zero restitution. Drops a Dynamic ball onto a Fixed floor with
    /// restitution=1.0; vertical velocity at the apex of the rebound should be
    /// substantially higher than the same setup with restitution=0.0.
    #[test]
    fn high_restitution_bounces_higher_than_zero_restitution() {
        fn peak_y_after_bounce(restitution: f32) -> f64 {
            let sim_arc = HookSim::new();
            let ball = Uuid::from_u128(1);
            let floor = Uuid::from_u128(2);
            // Both bodies share the restitution; Rapier averages contact-pair
            // material values (default `Average` rule), so setting it on both
            // pins the effective contact restitution.
            sim_arc.set(
                ball,
                EntitySpec {
                    shape: Some(RapierColliderShape::Ball(0.3)),
                    material: Some(RapierMaterial::new(0.0, restitution, 1.0)),
                    ..Default::default()
                },
            );
            sim_arc.set(
                floor,
                EntitySpec {
                    shape: Some(RapierColliderShape::Cuboid([20.0, 0.25, 20.0])),
                    body_kind: Some(RapierBodyKind::Fixed),
                    material: Some(RapierMaterial::new(0.0, restitution, 1.0)),
                    ..Default::default()
                },
            );
            let config = RapierConfig {
                gravity: [0.0, -9.81, 0.0],
                ..Default::default()
            };
            let sim = RapierClusterSim::with_rapier_sim(
                sim_arc.clone() as Arc<dyn RapierClusterSimulation>,
                config,
            );

            let mut entities = HashMap::new();
            entities.insert(
                ball,
                mk_entry(ball, Vec3::new(0.0, 3.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            );
            entities.insert(
                floor,
                mk_entry(floor, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            );

            // Drop time ≈ √(2·3/9.81) ≈ 0.78 s ≈ 16 cluster ticks. After tick
            // ~20 the ball has impacted; from there, track the rebound peak.
            // Bouncy (r=1) rebounds toward y≈3; dead (r=0) plateaus at floor.
            let mut peak_after_impact = f64::NEG_INFINITY;
            for tick in 0..80 {
                step_once(&sim, &mut entities, tick + 1, CLUSTER_DT);
                if tick >= 20 {
                    let y = entities.get(&ball).unwrap().position.y;
                    if y > peak_after_impact {
                        peak_after_impact = y;
                    }
                }
            }
            peak_after_impact
        }

        let bouncy = peak_y_after_bounce(1.0);
        let dead = peak_y_after_bounce(0.0);
        // Bouncy rebounds to a meaningful height above where the dead ball
        // came to rest. Generous margin (1.0 m) for substep losses.
        assert!(
            bouncy > dead + 1.0,
            "bouncy post-impact peak {} must exceed dead post-impact peak {} by > 1.0",
            bouncy,
            dead
        );
    }

    /// **#120-T5**: density change affects mass-derived collision response.
    /// Inspect the body's mass after spawn — for a unit-radius ball with the
    /// default density formula, doubling density doubles mass.
    #[test]
    fn density_changes_body_mass() {
        fn mass_for_density(density: f32) -> f32 {
            let sim_arc = HookSim::new();
            let id = Uuid::from_u128(1);
            sim_arc.set(
                id,
                EntitySpec {
                    shape: Some(RapierColliderShape::Ball(1.0)),
                    material: Some(RapierMaterial::new(0.0, 0.0, density)),
                    ..Default::default()
                },
            );
            let sim = RapierClusterSim::with_rapier_sim(
                sim_arc.clone() as Arc<dyn RapierClusterSimulation>,
                RapierConfig::default(),
            );
            let mut entities = HashMap::new();
            entities.insert(
                id,
                mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            );
            step_once(&sim, &mut entities, 1, CLUSTER_DT);
            let state = sim.state.lock().unwrap();
            let h = *state.handles.get(&id).unwrap();
            state.bodies.get(h).unwrap().mass()
        }
        let m1 = mass_for_density(1.0);
        let m2 = mass_for_density(2.0);
        assert!(
            (m2 / m1 - 2.0).abs() < 1e-3,
            "doubling density should double mass: m1 = {}, m2 = {}",
            m1,
            m2
        );
    }

    /// **#120-T6**: collision groups filter contacts. Two overlapping bodies
    /// in non-overlapping groups must not generate any contact events. Same
    /// pair with default groups produces contacts (sanity-check baseline).
    #[test]
    fn non_overlapping_collision_groups_filter_contacts() {
        // Group setup: A is in GROUP_1, filters only GROUP_1 (i.e. won't see B).
        //              B is in GROUP_2, filters only GROUP_2 (won't see A).
        let sim_arc = HookSim::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        sim_arc.set(
            a,
            EntitySpec {
                shape: Some(RapierColliderShape::Ball(0.5)),
                groups: Some(RapierCollisionGroups::new(Group::GROUP_1, Group::GROUP_1)),
                ..Default::default()
            },
        );
        sim_arc.set(
            b,
            EntitySpec {
                shape: Some(RapierColliderShape::Ball(0.5)),
                groups: Some(RapierCollisionGroups::new(Group::GROUP_2, Group::GROUP_2)),
                ..Default::default()
            },
        );
        let sim = RapierClusterSim::with_rapier_sim(
            sim_arc.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );

        let mut entities = HashMap::new();
        // Significant overlap — without filtering this would produce contacts.
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(0.4, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );

        step_n(&sim, &mut entities, 5, CLUSTER_DT);

        let events = sim_arc.snapshot_events();
        assert!(
            !events
                .iter()
                .any(|e| (e.entity_a == a && e.entity_b == b)
                    || (e.entity_a == b && e.entity_b == a)),
            "non-overlapping groups should suppress contacts; got {:?}",
            events
        );
    }

    /// **#120-T7**: a sensor collider fires the contact event but does NOT
    /// produce physical pushback on the partner body. Without filtering, two
    /// overlapping balls would resolve apart; with one as a sensor, the
    /// non-sensor ball stays at its starting position.
    #[test]
    fn sensor_fires_event_without_pushback() {
        let sim_arc = HookSim::new();
        let trigger = Uuid::from_u128(1);
        let body = Uuid::from_u128(2);
        sim_arc.set(
            trigger,
            EntitySpec {
                shape: Some(RapierColliderShape::Ball(0.5)),
                body_kind: Some(RapierBodyKind::Fixed),
                is_sensor: Some(true),
                ..Default::default()
            },
        );
        sim_arc.set(
            body,
            EntitySpec {
                shape: Some(RapierColliderShape::Ball(0.5)),
                ..Default::default()
            },
        );
        let sim = RapierClusterSim::with_rapier_sim(
            sim_arc.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );

        let mut entities = HashMap::new();
        let body_start = Vec3::new(0.4, 0.0, 0.0);
        entities.insert(
            trigger,
            mk_entry(trigger, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(body, mk_entry(body, body_start, Vec3::new(0.0, 0.0, 0.0)));

        step_n(&sim, &mut entities, 5, CLUSTER_DT);

        // Contact event fired.
        let events = sim_arc.snapshot_events();
        assert!(
            events
                .iter()
                .any(|e| (e.entity_a == trigger && e.entity_b == body)
                    || (e.entity_a == body && e.entity_b == trigger)),
            "sensor must still surface contact event; got {:?}",
            events
        );

        // No pushback — body stayed at its start.
        let p = entities.get(&body).unwrap().position;
        assert!(
            close(p.x, body_start.x, 1e-3),
            "sensor produced pushback: x moved from {} to {}",
            body_start.x,
            p.x
        );
    }

    /// **#120-T8**: every spawn-time hook is called exactly once per entity
    /// at first-sight. Subsequent ticks do not re-invoke the hooks.
    #[test]
    fn all_hooks_called_exactly_once_per_entity() {
        let sim_arc = HookSim::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let sim = RapierClusterSim::with_rapier_sim(
            sim_arc.clone() as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        // a spawns on tick 1; b spawns on tick 4 (later first-sight).
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 3, CLUSTER_DT);

        entities.insert(
            b,
            mk_entry(b, Vec3::new(50.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 4, CLUSTER_DT);

        // Hook call counts: each of the five hooks must have been called
        // exactly once for each of the two entities.
        for hook in [
            "collider_for",
            "body_kind_for",
            "material_for",
            "collision_groups_for",
            "is_sensor",
        ] {
            assert_eq!(
                sim_arc.count(hook, a),
                1,
                "{hook} called {} times for entity a",
                sim_arc.count(hook, a)
            );
            assert_eq!(
                sim_arc.count(hook, b),
                1,
                "{hook} called {} times for entity b",
                sim_arc.count(hook, b)
            );
        }
    }

    // ─── in-tick imperative ops (#121): impulses / forces / set_* / queries / joints ──

    /// Generic test fixture: each tick, run the user-provided closure against
    /// the [`PhysicsHandle`] and the current tick number. The closure may use
    /// interior mutability (e.g. `Mutex<Vec<RaycastHit>>`) to record results.
    struct ScriptedSim<F>
    where
        F: Fn(&mut PhysicsHandle, u64) + Send + Sync,
    {
        action: F,
    }

    impl<F> RapierClusterSimulation for ScriptedSim<F>
    where
        F: Fn(&mut PhysicsHandle, u64) + Send + Sync,
    {
        fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
            (self.action)(&mut ctx.physics, ctx.tick);
        }
    }

    fn run_with_action<F>(
        config: RapierConfig,
        action: F,
        seed_entities: Vec<(Uuid, EntityStateEntry)>,
        ticks: u64,
    ) -> (
        Arc<ScriptedSim<F>>,
        RapierClusterSim,
        HashMap<Uuid, EntityStateEntry>,
    )
    where
        F: Fn(&mut PhysicsHandle, u64) + Send + Sync + 'static,
    {
        let sim_arc = Arc::new(ScriptedSim { action });
        let sim = RapierClusterSim::with_rapier_sim(
            sim_arc.clone() as Arc<dyn RapierClusterSimulation>,
            config,
        );
        let mut entities: HashMap<Uuid, EntityStateEntry> = seed_entities.into_iter().collect();
        step_n(&sim, &mut entities, ticks, CLUSTER_DT);
        (sim_arc, sim, entities)
    }

    /// **#121-T1**: `apply_impulse` produces a linvel change of `impulse / mass`.
    /// With unit-density ball of radius 0.5 (mass ≈ 0.524), an impulse of
    /// (10, 0, 0) should yield a positive linvel along x. Applied at tick 2
    /// because the entity is spawned during tick 1's spawn loop, after on_tick.
    #[test]
    fn apply_impulse_changes_linvel_proportional_to_impulse_over_mass() {
        let id = Uuid::from_u128(1);
        let action = move |physics: &mut PhysicsHandle, tick: u64| {
            if tick == 2 {
                physics.apply_impulse(id, Vec3::new(10.0, 0.0, 0.0));
            }
        };
        let (_sim_arc, _sim, entities) = run_with_action(
            RapierConfig::default(),
            action,
            vec![(
                id,
                mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            )],
            2,
        );
        // After tick 2, body.linvel reflects the impulse (touched_linvel skip
        // prevents the per-tick set_linvel sync from clobbering it). For unit-
        // density ball (mass ≈ 0.524), 10 N·s impulse yields vx ≈ 19.
        let v = entities.get(&id).unwrap().velocity;
        assert!(
            v.x > 5.0,
            "expected positive x velocity from impulse; got {:?}",
            v
        );
    }

    /// **#121-T2**: `apply_force` sustained over multiple ticks produces
    /// approximately linear velocity growth (constant acceleration).
    #[test]
    fn apply_force_over_multiple_ticks_produces_acceleration() {
        let id = Uuid::from_u128(1);
        let action = move |physics: &mut PhysicsHandle, _tick: u64| {
            physics.apply_force(id, Vec3::new(5.0, 0.0, 0.0));
        };
        let (_sim_arc, _sim, entities) = run_with_action(
            RapierConfig::default(),
            action,
            vec![(
                id,
                mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            )],
            20,
        );
        // After 20 cluster ticks (1 s), constant force should have produced
        // significant positive velocity along x.
        let v = entities.get(&id).unwrap().velocity;
        assert!(
            v.x > 1.0,
            "expected positive x velocity from sustained force; got {:?}",
            v
        );
    }

    /// **#121-T3**: `set_translation` teleports the body; the new position
    /// propagates to `entity.position` via `sync_outputs`.
    #[test]
    fn set_translation_teleports_and_propagates_to_entity_position() {
        let id = Uuid::from_u128(1);
        let action = move |physics: &mut PhysicsHandle, tick: u64| {
            if tick == 2 {
                physics.set_translation(id, Vec3::new(100.0, 50.0, -25.0));
            }
        };
        let (_sim_arc, _sim, entities) = run_with_action(
            RapierConfig::default(),
            action,
            vec![(
                id,
                mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            )],
            3,
        );
        let p = entities.get(&id).unwrap().position;
        assert!(close(p.x, 100.0, 0.5), "x = {}", p.x);
        assert!(close(p.y, 50.0, 0.5), "y = {}", p.y);
        assert!(close(p.z, -25.0, 0.5), "z = {}", p.z);
    }

    /// **#121-T4**: imperative `set_linvel` is NOT clobbered by the per-tick
    /// `entity.velocity` → `body.linvel` sync. The user wrote `entity.velocity = (1,0,0)`
    /// before this tick (seeded), but the imperative call overrides to (5,0,0).
    /// After the step, body's velocity must be the imperative value.
    #[test]
    fn set_linvel_imperative_overrides_per_tick_sync() {
        let id = Uuid::from_u128(1);
        let action = move |physics: &mut PhysicsHandle, tick: u64| {
            if tick == 2 {
                // entity.velocity is (1,0,0); imperatively force it to (5,0,0).
                physics.set_linvel(id, Vec3::new(5.0, 0.0, 0.0));
            }
        };
        let (_sim_arc, sim, entities) = run_with_action(
            RapierConfig::default(),
            action,
            vec![(
                id,
                mk_entry(id, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
            )],
            2,
        );
        // The body should reflect the imperative linvel post-tick. Sync_outputs
        // writes body.linvel to entity.velocity at end of tick.
        let v = entities.get(&id).unwrap().velocity;
        assert!(
            close(v.x, 5.0, 0.1),
            "imperative set_linvel was clobbered; expected vx ≈ 5.0, got {}",
            v.x
        );
        let _ = sim;
    }

    /// **#121-T5**: `raycast` finds the entity collider in the ray's path and
    /// returns its `entity_id` plus the time-of-impact / hit point.
    #[test]
    fn raycast_hits_collider_in_line() {
        let target = Uuid::from_u128(7);
        let result: Arc<Mutex<Option<RaycastHit>>> = Arc::new(Mutex::new(None));
        let result_clone = result.clone();
        let action = move |physics: &mut PhysicsHandle, tick: u64| {
            if tick == 2 {
                // Ray from origin shooting +X. Target ball at (10,0,0) radius 0.5.
                let hit = physics.raycast(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 20.0);
                *result_clone.lock().unwrap() = hit;
            }
        };
        let (_sim_arc, _sim, _entities) = run_with_action(
            RapierConfig::default(),
            action,
            vec![(
                target,
                mk_entry(target, Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            )],
            2,
        );
        let hit = result
            .lock()
            .unwrap()
            .expect("ray should hit the target collider");
        assert_eq!(hit.entity_id, target);
        // Target at x=10, ball radius 0.5 → first hit at x≈9.5.
        assert!(
            (hit.time_of_impact - 9.5).abs() < 0.5,
            "expected toi≈9.5, got {}",
            hit.time_of_impact
        );
    }

    /// **#121-T6**: `raycast` returns `None` when no collider is in the ray's path.
    #[test]
    fn raycast_misses_when_no_collider_in_line() {
        let target = Uuid::from_u128(7);
        let result: Arc<Mutex<Option<RaycastHit>>> = Arc::new(Mutex::new(None));
        let recorded: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let result_clone = result.clone();
        let recorded_clone = recorded.clone();
        let action = move |physics: &mut PhysicsHandle, tick: u64| {
            if tick == 2 {
                // Target is at +X (10,0,0); ray shoots straight up — should miss.
                *result_clone.lock().unwrap() =
                    physics.raycast(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 50.0);
                *recorded_clone.lock().unwrap() = true;
            }
        };
        let (_sim_arc, _sim, _entities) = run_with_action(
            RapierConfig::default(),
            action,
            vec![(
                target,
                mk_entry(target, Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            )],
            2,
        );
        assert!(*recorded.lock().unwrap(), "raycast call did not happen");
        assert!(
            result.lock().unwrap().is_none(),
            "raycast should miss; got {:?}",
            result.lock().unwrap()
        );
    }

    /// **#121-T7**: `intersections_with_shape` returns the entity ids whose
    /// colliders overlap a query shape positioned in the world.
    #[test]
    fn intersections_with_shape_returns_overlapping_entities() {
        let near = Uuid::from_u128(1);
        let far = Uuid::from_u128(2);
        let result: Arc<Mutex<Vec<Uuid>>> = Arc::new(Mutex::new(Vec::new()));
        let result_clone = result.clone();
        let action = move |physics: &mut PhysicsHandle, tick: u64| {
            if tick == 2 {
                // Sphere of radius 5 at origin should hit `near` (at 2,0,0)
                // and miss `far` (at 100,0,0).
                let hits = physics.intersections_with_shape(
                    &RapierColliderShape::Ball(5.0),
                    Vec3::new(0.0, 0.0, 0.0),
                );
                *result_clone.lock().unwrap() = hits;
            }
        };
        let (_sim_arc, _sim, _entities) = run_with_action(
            RapierConfig::default(),
            action,
            vec![
                (
                    near,
                    mk_entry(near, Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
                ),
                (
                    far,
                    mk_entry(far, Vec3::new(100.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
                ),
            ],
            2,
        );
        let hits = result.lock().unwrap();
        assert!(hits.contains(&near), "near should be hit; got {:?}", hits);
        assert!(
            !hits.contains(&far),
            "far should NOT be hit; got {:?}",
            hits
        );
    }

    /// **#121-T8**: a Fixed joint between two entities holds them at fixed
    /// relative positions. After many ticks of the second entity having a
    /// non-zero velocity, the relative offset should be roughly constant.
    #[test]
    fn fixed_joint_holds_entities_at_fixed_offset() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let joint_created: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let joint_created_clone = joint_created.clone();
        let action = move |physics: &mut PhysicsHandle, tick: u64| {
            if tick == 2 && !*joint_created_clone.lock().unwrap() {
                // Anchor at the midpoint between the two bodies (bodies at
                // distance 2 along x, anchor at +1 in A's frame, -1 in B's frame).
                let result = physics.create_joint(
                    a,
                    b,
                    JointSpec::Fixed {
                        local_anchor_a: Vec3::new(1.0, 0.0, 0.0),
                        local_anchor_b: Vec3::new(-1.0, 0.0, 0.0),
                    },
                );
                assert!(result.is_some(), "create_joint should succeed");
                *joint_created_clone.lock().unwrap() = true;
            }
        };
        let (_sim_arc, _sim, entities) = run_with_action(
            RapierConfig::default(),
            action,
            vec![
                (
                    a,
                    mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
                ),
                (
                    b,
                    mk_entry(b, Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
                ),
            ],
            30,
        );
        // Without a joint, b would drift to ~x=3.5 by tick 30 (~1.5s × 1 unit/s, minus
        // collision contributions). With the joint, a and b should still be ~2 apart.
        let pa = entities.get(&a).unwrap().position;
        let pb = entities.get(&b).unwrap().position;
        let dx = (pb.x - pa.x).abs();
        assert!(
            (dx - 2.0).abs() < 0.5,
            "fixed joint failed: |b.x - a.x| = {} (expected ~2.0)",
            dx
        );
    }

    /// **#121-T9**: when an entity in a joint is despawned, the joint is
    /// cleaned up automatically. Subsequent `remove_joint` returns `false`.
    #[test]
    fn despawn_cleans_up_attached_joints() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let joint_id_holder: Arc<Mutex<Option<JointId>>> = Arc::new(Mutex::new(None));
        let joint_id_clone = joint_id_holder.clone();
        let despawn_signal: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let despawn_signal_clone = despawn_signal.clone();
        let remove_after_despawn_result: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
        let remove_clone = remove_after_despawn_result.clone();

        struct DespawnSim {
            a: Uuid,
            joint_id_holder: Arc<Mutex<Option<JointId>>>,
            despawn_signal: Arc<Mutex<bool>>,
            remove_after_despawn_result: Arc<Mutex<Option<bool>>>,
            other: Uuid,
        }
        impl RapierClusterSimulation for DespawnSim {
            fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
                if ctx.tick == 2 {
                    let id = ctx.physics.create_joint(
                        self.a,
                        self.other,
                        JointSpec::Fixed {
                            local_anchor_a: Vec3::new(0.5, 0.0, 0.0),
                            local_anchor_b: Vec3::new(-0.5, 0.0, 0.0),
                        },
                    );
                    *self.joint_id_holder.lock().unwrap() = id;
                }
                if ctx.tick == 3 {
                    ctx.pending_removals.push(self.a);
                    *self.despawn_signal.lock().unwrap() = true;
                }
                if ctx.tick == 5 {
                    if let Some(j) = *self.joint_id_holder.lock().unwrap() {
                        let r = ctx.physics.remove_joint(j);
                        *self.remove_after_despawn_result.lock().unwrap() = Some(r);
                    }
                }
            }
        }

        let sim_arc = Arc::new(DespawnSim {
            a,
            other: b,
            joint_id_holder: joint_id_clone,
            despawn_signal: despawn_signal_clone,
            remove_after_despawn_result: remove_clone,
        });
        let sim = RapierClusterSim::with_rapier_sim(
            sim_arc as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        entities.insert(
            a,
            mk_entry(a, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        entities.insert(
            b,
            mk_entry(b, Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
        );
        step_n(&sim, &mut entities, 5, CLUSTER_DT);

        assert!(
            *despawn_signal.lock().unwrap(),
            "despawn signal should have fired"
        );
        let id = joint_id_holder.lock().unwrap();
        assert!(id.is_some(), "joint should have been created");
        let r = remove_after_despawn_result.lock().unwrap();
        assert_eq!(
            *r,
            Some(false),
            "remove_joint after despawn should return false (joint already gone)"
        );
    }

    /// **#121-T10**: operations on a missing `entity_id` return `false` /
    /// `None` without panicking.
    #[test]
    fn operations_on_missing_entity_id_no_panic() {
        let unknown = Uuid::from_u128(999);
        let action = move |physics: &mut PhysicsHandle, tick: u64| {
            if tick == 1 {
                assert!(!physics.apply_impulse(unknown, Vec3::new(1.0, 0.0, 0.0)));
                assert!(!physics.apply_force(unknown, Vec3::new(1.0, 0.0, 0.0)));
                assert!(!physics.apply_torque_impulse(unknown, Vec3::new(1.0, 0.0, 0.0)));
                assert!(!physics.set_translation(unknown, Vec3::new(0.0, 0.0, 0.0)));
                assert!(!physics.set_linvel(unknown, Vec3::new(0.0, 0.0, 0.0)));
                assert!(!physics.set_angvel(unknown, Vec3::new(0.0, 0.0, 0.0)));
                assert!(physics.linvel(unknown).is_none());
                assert!(physics.angvel(unknown).is_none());
                assert!(!physics.wake(unknown));
                assert!(!physics.sleep(unknown));
                let joint = physics.create_joint(
                    unknown,
                    unknown,
                    JointSpec::Fixed {
                        local_anchor_a: Vec3::new(0.0, 0.0, 0.0),
                        local_anchor_b: Vec3::new(0.0, 0.0, 0.0),
                    },
                );
                assert!(joint.is_none());
            }
        };
        let (_sim_arc, _sim, _entities) =
            run_with_action(RapierConfig::default(), action, vec![], 1);
    }

    /// **#121-T11**: imperative ops on a `Fixed` body silently no-op and
    /// return `false`; the body's state is unchanged.
    #[test]
    fn imperative_ops_on_fixed_body_are_no_ops() {
        let id = Uuid::from_u128(1);
        let result: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
        let result_clone = result.clone();
        struct FixedSim {
            id: Uuid,
            result: Arc<Mutex<Vec<bool>>>,
        }
        impl RapierClusterSimulation for FixedSim {
            fn on_tick(&self, ctx: &mut RapierClusterTickContext<'_>) {
                if ctx.tick == 2 {
                    let r1 = ctx
                        .physics
                        .apply_impulse(self.id, Vec3::new(100.0, 0.0, 0.0));
                    let r2 = ctx.physics.apply_force(self.id, Vec3::new(100.0, 0.0, 0.0));
                    let r3 = ctx.physics.set_linvel(self.id, Vec3::new(50.0, 0.0, 0.0));
                    self.result.lock().unwrap().extend([r1, r2, r3]);
                }
            }
            fn body_kind_for(
                &self,
                _entry: &EntityStateEntry,
                _config: &RapierConfig,
            ) -> RapierBodyKind {
                RapierBodyKind::Fixed
            }
        }
        let sim_arc = Arc::new(FixedSim {
            id,
            result: result_clone,
        });
        let sim = RapierClusterSim::with_rapier_sim(
            sim_arc as Arc<dyn RapierClusterSimulation>,
            RapierConfig::default(),
        );
        let mut entities = HashMap::new();
        let start = Vec3::new(5.0, 5.0, 5.0);
        entities.insert(id, mk_entry(id, start, Vec3::new(0.0, 0.0, 0.0)));
        step_n(&sim, &mut entities, 4, CLUSTER_DT);

        // All three imperative ops should have returned false.
        let results = result.lock().unwrap();
        assert_eq!(*results, vec![false, false, false]);
        // Body should not have moved.
        let p = entities.get(&id).unwrap().position;
        assert!(close(p.x, start.x, 1e-6) && close(p.y, start.y, 1e-6));
    }
}
