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

## 7. Terrain — game owns storage and authoring; Arcane owns the loading interface

Terrain is the only thing in the world that's *not* an entity. Three terrain shapes need to work in Arcane, all through the same interface:

| Terrain shape | Storage | Mutability | Example games |
|---|---|---|---|
| **Static / mesh** | Object storage (S3, CDN, on-disk asset bundle) | Read-only at runtime | UE level, Unity scene, glTF import |
| **Voxel** | **SpacetimeDB** (voxel grid is durable state — every block edit persists) | Mutable per-block, durable across sessions | Minecraft, Valheim, Astroneer |
| **Procedural / hybrid** | Seed in SpacetimeDB; geometry generated on demand; modifications stored as durable diffs | Effectively mutable via overrides | No Man's Sky, procedural sandboxes |

If a game has destructible terrain that's purely entity-flavored (a wall a player can knock down), that's still an entity, not a terrain edit. Voxel terrain is genuinely terrain — it has no per-block entity_id, no per-block durable rows for entity-style state. Voxel chunks ARE durable rows, just at chunk granularity, not block granularity.

### Arcane provides — the MapProvider interface

The cluster runtime owns chunk loading. The game implements a `MapProvider` (per-engine plugin name) that the runtime calls:

```rust
// Rapier-Rust shape; UE/Unity/Godot have parallel APIs in their native languages.
pub trait RapierMapProvider: Send + Sync {
    /// Compute which chunks need to be loaded given the cluster's currently
    /// owned entity positions. Pure function of input; called per cluster
    /// tick (or on entity arrival/departure events).
    fn chunks_in_range(&self, entity_positions: &[Vec3]) -> Vec<ChunkId>;

    /// Fetch the collision geometry for a chunk. Implementation reads from
    /// wherever the game stores it (SpacetimeDB voxel chunks, object storage
    /// mesh bundles, embedded assets, procedural generators) — game's choice.
    fn load_chunk(&self, chunk_id: ChunkId) -> Result<ChunkCollision, MapError>;
}

pub enum ChunkCollision {
    TriMesh { vertices: Vec<Vec3>, indices: Vec<[u32; 3]> },
    HeightField { width: usize, height: usize, samples: Vec<f32> },
    // Voxel games typically convert to TriMesh via greedy meshing
    // before returning, or expose per-block boxes if blocks are sparse.
}
```

Per the per-engine API discipline, this is one of many parallel APIs — the UE plugin defines `IArcaneUnrealMapProvider` returning UE-native collision data; Unity does the same with Unity-native types; etc. Different language, same conceptual contract.

### Arcane does NOT provide

- A map asset format. Game decides.
- An authoring tool. Game uses its engine's editor (UE, Unity, Godot, Blender, custom) or generates procedurally.
- A default storage backend. Game picks SpacetimeDB / object storage / on-disk / procedural / hybrid.
- Voxel-specific or mesh-specific support. The interface is uniform; the implementation differs per game.

### Where things live (typical layout)

| Data | Storage |
|---|---|
| Static map content (mesh files, prebaked geometry) | Object storage / asset bundle / on-disk — game's choice |
| Voxel terrain content | SpacetimeDB (durable, mutable, chunk granularity) |
| Map manifest (chunk catalog, version pointers) | SpacetimeDB row(s) — small, always available, used by cluster startup |
| Mutable per-chunk state (destruction events on a mesh chunk, voxel diffs) | SpacetimeDB rows tied to `chunk_id` |
| Per-chunk **entities** (placed structures, drops) | SpacetimeDB — already entities, already durable |

**Game developers never insert terrain geometry into physics by hand at runtime.** They author the map (in their engine's editor or as voxel data); they implement the `MapProvider`; the cluster runtime calls it. See [issue #119](https://github.com/brainy-bots/arcane/issues/119).

---

## 8. Conceptual contract vs. per-engine API

This document defines **conceptual contracts**: what an entity is, the body-kind taxonomy, the affinity-vs-spatial binding distinction, the terrain-vs-entities split. These are vocabulary and mental model, not a code library.

**The user-facing API for declaring entity properties is per-engine.** Arcane has multiple engine plugins (Rapier-Rust, UE, Unity, Godot — current and future). Each plugin exposes its own engine-native API for game developers, written in the engine's language and matching the engine's idioms.

| Engine | User-facing API for entities | Body-kind expression |
|---|---|---|
| **Rapier (Rust)** | Implement the `RapierClusterSimulation` trait; per-entity hooks return Rust enums | `RapierBodyKind` enum |
| **Unreal (C++)** | Subclass `AArcaneUnrealEntity` (extends `AActor`); plugin reads UE's native properties | `EComponentMobility` (UE-native) — **not** mirrored as a separate Arcane enum |
| **Unity (C#)** | Add `ArcaneUnityEntity` `MonoBehaviour` to a `GameObject`; plugin reads Unity's native properties | `Rigidbody.bodyType` + `Rigidbody.isKinematic` (Unity-native) |
| **Godot (GDScript / C#)** | Subclass `ArcaneGodotEntity` (Node3D base); plugin reads Godot's native node class | Choice of body class (`RigidBody3D` / `StaticBody3D` / `AnimatableBody3D` / `Area3D`) — Godot-native |

**There is no shared Arcane `BodyKind` enum across engines.** The conceptual taxonomy (Dynamic / KinematicPositionBased / KinematicVelocityBased / Fixed) is documented here as vocabulary; each plugin uses its own engine-native equivalent.

The same applies to colliders, materials, collision groups, sensor flags, joint specs, and contact events. **Each engine plugin has its own value types in its own language.** Rust plugins use Rust enums; UE plugins use UENUM / USTRUCT; Unity uses C# classes; Godot uses GDScript dictionaries or C# classes.

### What IS engine-neutral and shared

- **Wire format types** (`EntityStateEntry`, `EntityStateDelta`, postcard / arcane-wire bytes) — every cluster speaks the same protocol.
- **Manager / replication protocols** — HTTP join, Redis pub/sub channel layout, neighbor delta semantics.
- **Durable state schema invariant** — every entity has a SpacetimeDB row.
- **Conceptual vocabulary** — this document.
- **Industry-standard term cross-references** — to make cross-engine and cross-team conversations productive (§9 below).

### Why per-engine, not shared

- Different languages (C++, C#, GDScript, Rust). Code-level type sharing is impossible across language boundaries; "shared types" become four parallel re-implementations of the same enum.
- Different idioms. UE devs hate writing un-UE-like code; same for every engine community.
- Game devs already think in their engine's vocabulary. Layering a parallel Arcane vocabulary on top is friction without benefit.
- Cross-engine consistency that *matters* (wire format, manager protocol, durable state) is enforced where it must be — at the protocol layer.

---

## 9. Engine plugin pattern

The canonical shape of an Arcane engine plugin:

1. **Engine-native base class or interface** for game-side entities, named with the `Arcane{Engine}Entity` convention. Extends or implements engine-native types so the dev's code looks engine-native.
2. **Per-engine cluster runtime** that hosts the simulation tick and dispatches to the user's game logic. Listens to manager, publishes to Redis, broadcasts to WS clients — all in the engine's native language.
3. **Per-engine `MapProvider`** that the game implements for terrain loading.
4. **Per-engine in-tick imperative ops** (apply impulse, raycast, etc.) — engine-native types as inputs and outputs, but always **entity-keyed** never engine-handle-keyed (preserves the wire-format invariant).
5. **Wire-format byte-compatibility** with every other engine plugin. Reads / writes the exact same `EntityStateDelta` bytes that Rust clusters do.

For game devs targeting multiple engines (e.g., a UE-cluster premium tier and a Rapier-cluster mid tier of the same game), this means writing **N parallel game-logic codebases**, one per engine plugin. Each is engine-native, idiomatic, and uses that engine's full feature set. **Cross-engine consistency for game rules** (damage, drops, currency, anything that must be transactionally consistent) **lives in SpacetimeDB reducers** — called from every engine plugin's cluster binary, ensuring the rules are guaranteed identical.

The platform doesn't try to auto-port code. Devs choose how many tiers to support and write logic appropriate to each.

---

## 10. Cross-engine entity migration

Entities can migrate between cluster tiers running different engines (per the heterogeneous-tier vision in `#33` and the dynamic migration epic in `#34`). The migration mechanism is at cluster-process boundaries:

1. Source cluster releases authority over the entity. Its engine-native game logic stops ticking it. Its physics body for the entity is destroyed.
2. **Durable state in SpacetimeDB is the source of truth**, has been throughout. Source writes a final state on release.
3. Target cluster takes authority. Reads durable state. **Target's engine-native game logic** (a different codebase, possibly a different language) starts ticking the entity. Target's physics body is spawned at the entity's current position.
4. The entity's position / velocity / replicated user_data continue to flow through the wire format unchanged.

There is **no in-process engine switching**. The "function that runs physics for this engine" is the entire cluster binary written in that engine's language. The platform's job at migration time is the ownership-transfer protocol; the dev's job is to make sure their per-engine logic implementations preserve the entity's gameplay state across the swap.

Migration timing is `#34`'s scope (dynamic tier migration as a platform primitive). Static tier placement (an entity is born in a tier, stays there) is the simpler v1 case.

---

---

## 11. Cross-references

| Topic | Doc |
|---|---|
| Entity state buckets (where each field lives) | [four-bucket-state-model.md](four-bucket-state-model.md) |
| Physics integration shape (Unreal/Chaos and Rapier) | [physics-backends-and-unreal.md](physics-backends-and-unreal.md) |
| Affinity clustering / PGP | [interface-iclusteringmodel.md](interface-iclusteringmodel.md) |
| Replication channels and wire format | [interface-ireplicationchannel.md](interface-ireplicationchannel.md) |

---

## 12. Industry-standard terminology cross-references

When discussing Arcane's entity model with people from other engines:

- Arcane "Entity" ≈ Unreal `AActor`, Unity `GameObject`, Bevy ECS Entity, Godot `Node3D`.
- Arcane "Body kind = Fixed" ≈ Unreal Mobility=Static, Unity `Rigidbody.isKinematic`+`isStatic`, Rapier `RigidBodyType::Fixed`, PhysX `PxRigidStatic`.
- Arcane "Affinity-bound" ≈ migrating actor in seamless world meshing (Star Citizen / SpatialOS terminology).
- Arcane "Spatial-bound" ≈ chunk-owned actor / persistent-placeable (Conan Exiles, Ark, Valheim terminology).
- Arcane "Terrain" ≈ static mesh / level geometry / world chunks (universally understood).
