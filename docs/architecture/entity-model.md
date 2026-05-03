# Entity model

This document is the **canonical reference** for what an *entity* is in Arcane, what kinds exist, how they relate to physics simulation and clustering, and how they relate to the world's terrain. It complements [four-bucket-state-model.md](four-bucket-state-model.md) (which defines where an entity's *state* lives) and [physics-backends-and-unreal.md](physics-backends-and-unreal.md) (which defines how an entity's *body* is simulated).

---

## 1. The unified entity model

Arcane has **one** persistent-world concept: the **Entity**. Players, NPCs, projectiles, dropped items, placed structures, player-built walls — every persistent thing in the game world that can be addressed, replicated, simulated, or destroyed is an entity. There is **no separate** "Structure," "GameObject," "Tile Entity," "Placeable," or similar category at the platform level.

This consolidation matches modern engine practice. Unreal's `AActor`, Unity's `GameObject`, Bevy's ECS Entity, and Godot's `Node3D` all use a single base concept differentiated by configuration (body kind, components, mobility flags). Older designs that split persistent things into `Unit` vs `GameObject` (WoW), `Entity` vs `Tile Entity` (Minecraft), or `APawn` vs `AStaticMeshActor` (early Unreal) have not aged well — they bake assumptions ("static things never become dynamic," "dynamic things don't have durable state") into the type system that real games then have to fight.

Game developers using Arcane define their **own subclasses or type categories** of Entity to express game-specific behavior (e.g. `Player`, `NPC`, `Projectile`, `Wall`, `Chest`). The platform doesn't enumerate these. What the platform exposes is a small set of **per-entity hooks** that determine how each entity is treated by physics and clustering — body kind, collider shape, material, collision groups, sensor status. The game's subclasses pick the right hook return values for their purposes.

---

## 2. Universal entity properties

Every entity, regardless of kind, has the following:

| Property | Where it lives | Notes |
|---|---|---|
| **Stable ID** | `EntityStateEntry::entity_id` (UUID) | Survives cluster restarts, migrations, durable persistence. |
| **Cluster ownership** | `EntityStateEntry::cluster_id` | The cluster currently authoritative for this entity. May change at migration time. |
| **Position + velocity (spine pose)** | `EntityStateEntry::position` / `velocity` | Bucket 1 of the four-bucket state model. Replicated every tick. |
| **Replicated user data** | `EntityStateEntry::user_data` (JSON) | Bucket 2 — game-defined fields that must reach neighbors and clients. |
| **Cluster-local ephemeral data** | `EntityStateEntry::local_data` (JSON) | Bucket 3 — process-only scratch, not on the wire. |
| **Durable state** | SpacetimeDB row | Bucket 4. **Every entity has durable state in SpacetimeDB.** This is an invariant of the platform. |

The "every entity has SpacetimeDB durable state" invariant is what makes recovery possible: cluster crash, cluster restart, cluster migration, every kind of disruption can rehydrate the entity from its durable row. It also unifies the lifecycle — there is no class of persistent thing in the world that lives outside SpacetimeDB.

---

## 3. Two-axis classification

Entities differ along two orthogonal axes:

| | **Animate** (has AI / will / behavior) | **Inanimate** (no will, no AI) |
|---|---|---|
| **Moving** | Player running, NPC walking, NPC pathfinding | Thrown box, projectile, falling debris, ragdoll, dropped item still settling |
| **Stationary** | NPC standing idle, AFK player, sleeping NPC | Wall, foundation, placed structure, dropped item at rest, world boss anchor |

Two practical observations follow from this:

- **Animation** is a **game-logic** distinction. The game tracks whether an entity has AI / behavior / inputs. The platform doesn't care — it's expressed in `user_data` or in the game's own SpacetimeDB schema.
- **Motion state** is **dynamic**, not static. A wall sits at rest (stationary, inanimate); a player attack sends it falling (moving, inanimate); when the rubble settles it becomes stationary again. **Same entity, same body, just changing motion state.** Physics engines (including Rapier) handle this via *sleeping* — a body at rest is automatically marked inactive, costs essentially zero per tick, and wakes when something hits it.

The platform's job isn't to rigidly classify entities — it's to express the *physical* differences cleanly so that the game's semantic categories ride on top.

---

## 4. Physics body kinds

Physics layer differentiates entities by **body kind**, which is what Rapier (and other physics engines) actually need:

| Body kind | Used for | Per-tick cost | Migration semantics |
|---|---|---|---|
| **Dynamic** | Players, NPCs (when physics-driven), projectiles, thrown objects, ragdolls, debris | Full simulation | Affinity-bound (PGP) |
| **KinematicPositionBased** | Player-controlled characters with custom locomotion, moving platforms, elevators | Position controlled by game logic; physics doesn't apply forces | Affinity-bound (PGP) |
| **KinematicVelocityBased** | Some character controllers; physics-integrated game-controlled velocity | Mid-ground between Dynamic and KinematicPositionBased | Affinity-bound (PGP) |
| **Fixed** | Walls, foundations, placed structures, permanent world fixtures (when expressed as entities) | Skipped by the solver entirely; only AABB tracking in broadphase | **Spatial-bound** (see §6) |

Body kind is declared per-entity at first-sight via the `body_kind_for` hook on `RapierClusterSimulation` (and analogous hooks on future Unreal/Unity backends). Default is `Dynamic`.

**Sleeping bodies** are how a cluster with thousands of stationary entities stays cheap: a wall sleeps, a placed crate sleeps, an idle NPC's body sleeps. Cost is roughly proportional to *active* (awake) bodies, not to total entity count. The "cluster with no entities pays no simulation cost" invariant is preserved because Fixed bodies don't iterate at all and other body kinds sleep when at rest.

---

## 5. Subclass vs property-value polymorphism

The unified-entity model does **not** mandate how the game expresses kind-specific behavior in code. Two equally-valid approaches:

- **Subclass-style** (Java / C#-like): the game defines `Player`, `NPC`, `Wall`, `Chest` as separate types that each implement `RapierClusterSimulation` with their own per-entity hooks. The platform calls the right impl per entity.
- **Property-value-style** (functional / Bevy ECS-like): the game has a single `RapierClusterSimulation` impl that matches on a kind field in `user_data` (or on the entity's SpacetimeDB row) and returns the appropriate body kind, shape, etc.

Both patterns work. The platform exposes the same hooks; the game picks the structure. Subclass-style tends to be more ergonomic for games with a small fixed catalog of entity kinds; property-value-style tends to be cleaner for games with many kinds or runtime-configurable kinds.

---

## 6. Affinity-bound vs spatial-bound — the clustering distinction

Where the unified-entity model **does** introduce a real architectural distinction is at the **clustering** layer, not the physics layer.

- **Affinity-bound entities** migrate by social signal (PGP). A player and their party-mates end up on the same cluster because they interact frequently. A projectile fired by a player follows the player's affinity. Examples: most Dynamic and Kinematic entities.
- **Spatial-bound entities** are tied to their position. They do not migrate by affinity — they only "migrate" when ownership of the map chunk they sit in changes hands. A wall built in the eastern desert stays in whichever cluster owns the eastern-desert chunk; it does not follow the guild that built it when the guild goes raiding 10km away. Examples: most Fixed entities.

The physics body kind correlates with binding (Fixed → spatial; Dynamic / Kinematic → affinity), but **the clustering model needs explicit binding information to make migration decisions**, because the clustering layer doesn't reach into physics. The cleanest encoding is an explicit `binding: EntityBinding { Affinity, Spatial }` field on the durable state, separate from physics body kind. (This is its own piece of design work — see the related epic.)

A consequence of spatial-bound entities is that **clustering is not driven purely by entity affinity any more**. The cluster manager has to balance:

- *Affinity-bound entities* — placed by the social-affinity model.
- *Spatial-bound entities* — placed by chunk ownership; the manager must arrange for a cluster's chunk responsibility to align with the spatial-bound entities sitting in those chunks.

This is the same kind of reasoning the manager already does for player movement (assigning chunks based on player density), generalized to "spatial-bound entities also influence which chunks a cluster wants."

---

## 7. Terrain is NOT entities

This is the only thing in the world that's *not* an entity:

| Aspect | Entity | Terrain |
|---|---|---|
| Identity | Stable UUID | None — terrain has no identity beyond chunk addressing |
| State | SpacetimeDB durable + spine pose + replicated user_data + local_data | None — terrain is *content*, not *state* |
| Storage | SpacetimeDB | Map asset on disk |
| Replication | Per-tick deltas | Not replicated — clients have own copy of map assets |
| Authority / ownership | A specific cluster at a given moment | Whichever cluster currently has chunks loaded; non-authoritative — every cluster reads the same map data |
| Mutability at runtime | Fully mutable | Read-only |

If a game has destructible terrain (a hole punched in a wall, dirt mined out), that destruction is modeled as **entities**, not as edits to the terrain. A wall that can be destroyed is an entity with `Fixed` body kind and durable HP state. Mining a tunnel is the despawn of a series of voxel-entities, leaving the underlying terrain mesh intact. The terrain layer is read-only; everything mutable is expressed at the entity layer.

**Game developers do not insert terrain geometry into physics by hand.** The Arcane runtime is responsible for loading the right map collision into a cluster's physics world based on which entities the cluster currently owns and where they are. See [issue #119](https://github.com/brainy-bots/arcane/issues/119) for the terrain-loading epic.

---

## 8. Cross-references

| Topic | Doc |
|---|---|
| Entity state buckets (where each field lives) | [four-bucket-state-model.md](four-bucket-state-model.md) |
| Physics integration shape (Unreal/Chaos and Rapier) | [physics-backends-and-unreal.md](physics-backends-and-unreal.md) |
| Affinity clustering / PGP | [interface-iclusteringmodel.md](interface-iclusteringmodel.md) |
| Replication channels and wire format | [interface-ireplicationchannel.md](interface-ireplicationchannel.md) |

---

## 9. Industry-standard terminology cross-references

When discussing Arcane's entity model with people from other engines:

- Arcane "Entity" ≈ Unreal `AActor`, Unity `GameObject`, Bevy ECS Entity, Godot `Node3D`.
- Arcane "Body kind = Fixed" ≈ Unreal Mobility=Static, Unity `Rigidbody.isKinematic`+`isStatic`, Rapier `RigidBodyType::Fixed`, PhysX `PxRigidStatic`.
- Arcane "Affinity-bound" ≈ migrating actor in seamless world meshing (Star Citizen / SpatialOS terminology).
- Arcane "Spatial-bound" ≈ chunk-owned actor / persistent-placeable (Conan Exiles, Ark, Valheim terminology).
- Arcane "Terrain" ≈ static mesh / level geometry / world chunks (universally understood).
