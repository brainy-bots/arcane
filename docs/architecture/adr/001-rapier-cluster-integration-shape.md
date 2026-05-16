# ADR-001: Rapier cluster integration shape

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-05-03 |
| **Implemented in** | PR [`brainy-bots/arcane#123`](https://github.com/brainy-bots/arcane/pull/123); commits `131b439`, `664671c`, `6a9c4fe`, `f91522d`, `f0938c9`, `0f5cb7a`, `57df0fc` |
| **Related epic** | [`#8`](https://github.com/brainy-bots/arcane/issues/8) — Cluster physics backends — Unreal (Chaos) first, multi-engine path |
| **Related issues** | [`#117`](https://github.com/brainy-bots/arcane/issues/117), [`#118`](https://github.com/brainy-bots/arcane/issues/118), [`#120`](https://github.com/brainy-bots/arcane/issues/120), [`#121`](https://github.com/brainy-bots/arcane/issues/121), [`#122`](https://github.com/brainy-bots/arcane/issues/122) |

## Context

Epic [`#8`](https://github.com/brainy-bots/arcane/issues/8) names Unreal/Chaos as the first concrete physics backend and Rapier as the second. Rapier landed first as the in-process Rust path because Rapier-in-Rust avoids FFI and lets the integration share `arcane-infra`'s networking primitives directly; the lessons inform the harder Unreal-native path.

The decision space had three dimensions:

1. **Trait shape** — should the cluster runtime introduce a new `PhysicsBackend` trait, or should physics be expressed through the existing `ClusterSimulation` hook?
2. **Process shape** — does Rapier run as a sidecar process / library / FFI-wrapped engine?
3. **State exposure to user code** — does game logic see Rapier's `RigidBodySet` directly, or does it work through entity-keyed methods?

[`docs/architecture/physics-backends-and-unreal.md`](../physics-backends-and-unreal.md) §7 (Multi-backend path) anticipated a separate crate per backend. The Rapier work refined this: the actual decision is finer than "separate crate or not."

## Decision

### 1. Composition over inheritance — no new trait

`RapierClusterSim` **is a `ClusterSimulation`** (the existing hook trait in `arcane-core`) that **wraps a user-provided `ClusterSimulation` (or the new `RapierClusterSimulation` for richer use cases)**. The cluster runtime calls a single `ClusterSimulation::on_tick(...)`; what differs by backend is which struct is passed.

```text
                                         pure-Rapier path
ClusterSimulation                        ─────────────────
       ▲                                 RapierClusterSim::new(None, config)
       │
       ├── (user impl)                   wrapped-user-sim path
       │                                 ────────────────────
       └── RapierClusterSim ◄── wraps ── Arc<dyn ClusterSimulation>  (V1 user)
                              ◄── or ── Arc<dyn RapierClusterSimulation>  (V2 user)
```

**No new `PhysicsBackend` trait was introduced.** The `ClusterSimulation` trait is unchanged. `RapierClusterSimulation` is a *sibling* trait in `arcane-infra::rapier_cluster` (gated on the `rapier-cluster` Cargo feature) for users who want extended Rapier-specific surface (contact events, per-entity collider shapes); it is *not* a replacement.

### 2. In-process Rust library — no sidecar, no FFI

Rapier 0.32 is added as a Cargo `optional = true` dependency on `arcane-infra` behind feature `rapier-cluster`. The Rapier physics step runs inside `RapierClusterSim::on_tick`, in the same process and same thread as the cluster runtime. No IPC; no FFI; no separate binary.

Vanilla `cargo build -p arcane-infra` pulls **zero** `rapier3d` into the dep tree. The feature gate is the standard Rust pattern for optional heavy deps; no separate crate / repo / build target required.

### 3. Single `RapierState` mutex; user code never touches `RigidBodySet` directly

`RapierState` (private) owns `RigidBodySet`, `ColliderSet`, `IslandManager`, etc., behind a `Mutex` (required because `ClusterSimulation::on_tick` takes `&self`). The lock is held *during* user `on_tick` (single-threaded by construction; cluster runner is single-threaded per cluster).

User-facing API surfaces only **entity-keyed operations**: `apply_impulse(entity_id, impulse)`, `raycast(...)`, `set_translation(entity_id, position)` (planned in `#121`). Raw `&mut RigidBodySet` is never lent to user code. This preserves the wire-format invariant — no off-spine bodies that would lack `entity_id` and break replication.

### 4. Per-entity hooks are spawn-time, called once per entity

`collider_for(entry, config) -> RapierColliderShape` (and the planned spawn-time hooks in `#120`: `body_kind_for`, `material_for`, `is_sensor`, `collision_groups_for`) are called **exactly once at first-sight spawn**. After spawn, the body's shape and other per-entity properties are fixed. Mid-life shape change requires despawn-and-respawn — the same escape hatch as for any other Rapier-specific feature game devs may need.

### 5. Velocity in / position out contract

- `entity.velocity` is **intent-in.** Wrapper reads it at first-sight to seed the body's `linvel`; subsequent ticks read it as the per-tick velocity intent.
- `entity.position` is **output-only after first-sight spawn.** Wrapper reads `entity.position` exactly once when spawning; afterwards, Rapier owns position. User writes to `entity.position` during `on_tick` are silently overwritten by Rapier's post-step output.
- This contract is enforced by tests — see `position_writes_from_user_are_overwritten_by_rapier`.

### 6. Contact events surface with one-tick delay

Rapier emits `Started` / `Stopped` events during the physics step. These are buffered into `RapierState::pending_contact_events` and surfaced in **the next tick's** `RapierClusterTickContext::contact_events`. User logic always runs *before* physics each tick; one-tick delay is by design (intent before output).

Contact events on despawned bodies are **not** surfaced to the contact partner — when an entity is removed via `pending_removals`, its collider is dropped from the reverse map before the post-step event drain. Partners detect the loss via the entity map (the entity is gone), not via a `Stopped` contact event. Documented + tested.

### 7. Public API is `#[non_exhaustive]` from day one

`RapierColliderShape`, `ContactEvent`, `RapierClusterTickContext`, and `RapierConfig` are all `#[non_exhaustive]` — adding fields or variants in future versions is not a SemVer break. Codifies the API-stability discipline before any external user touches the surface.

## Alternatives considered

### A. Separate crate `arcane-physics-rapier` with its own `PhysicsBackend` trait

Rejected. Per `physics-backends-and-unreal.md` §7's earlier framing this looked clean, but in practice:

- Forced an artificial split between `arcane-infra` (networking) and a Rapier-specific crate that would import from it anyway.
- A new `PhysicsBackend` trait would have duplicated `ClusterSimulation`'s responsibility (per-tick hook on the entity map).
- Cargo feature flag inside `arcane-infra` achieves the same dependency isolation with less ceremony.

The original framing in `#8` is now updated to reflect the actual landed pattern.

### B. Sidecar process — Rust cluster + Rapier-in-separate-binary

Rejected. Rapier is a Rust library; running it in a separate process requires IPC for every per-tick state transfer (entity positions in, body positions out). At 20 Hz cluster tick × 1000 entities, that's 20,000 IPC messages per second per cluster. The latency overhead destroys the per-tick cost advantage.

The integration shape from `#8` §"Integration shapes" #1 (Rust cluster + Unreal sidecar) was always meant for cases where physics genuinely cannot run in-process (Unreal/Chaos has lifecycle requirements that make in-process FFI hostile). Rapier doesn't have those requirements.

### C. Direct `&mut RigidBodySet` exposure to user code

Rejected. Tempting because it gives users the full Rapier API for free, but:

- **Off-spine bodies** — user could insert bodies without `entity_id`s; those don't replicate; they're invisible to neighbor clusters; they die with the cluster process. Footgun: developers using off-spine bodies for gameplay state would silently break replication invariants.
- **Cross-cluster joints** — user could create joints between two entities in this cluster; if either entity migrates to another cluster, the joint becomes invalid. Without explicit lifecycle management, this is a silent correctness bug.
- **Wire-format invariant erosion** — direct handle access bypasses entity-id-keyed operations, breaking the assumption that everything in physics has a wire-format counterpart.

The wrapped, entity-keyed API gives users every Rapier capability that game logic actually needs without these footguns. Capabilities not yet wrapped (e.g., compound colliders, mesh colliders, contact-force events) are tracked in `#122` and added as games need them.

### D. Engine-neutral physics types in `arcane-core` shared across all backends

Rejected (after a brief attempt). Promoting `RapierBodyKind`, `RapierColliderShape`, `RapierMaterial` etc. into engine-neutral types in `arcane-core` looked clean for documentation but:

- UE/Unity/Godot plugins are written in different languages (C++, C#, GDScript). Rust types in `arcane-core` aren't reachable from those plugins; each plugin would have its own parallel re-implementation.
- Engine-native types (UE `Mobility`, Unity `Rigidbody.bodyType`, Godot subclass-per-kind) are what game devs already know. Layering a parallel Arcane vocabulary on top is friction without benefit.
- Cross-engine consistency that *matters* (wire format, manager protocol, durable state) is enforced where it must be — at the protocol layer, not the user-facing-API layer.

The decision lives in [`entity-model.md`](../entity-model.md) §8 and [`project_per_engine_api_pattern.md`](../../../../.claude/projects/-mnt-e-code-pgp-demo/memory/project_per_engine_api_pattern.md) memory. Each plugin defines its own engine-native value types; only the wire format is shared.

## Verification

| Check | Result |
|---|---|
| Vanilla `cargo build -p arcane-infra` | Compiles; **0** `rapier3d` references in dep tree |
| `cargo build -p arcane-infra --features rapier-cluster --bins` | Compiles |
| Vanilla `cargo test -p arcane-infra` | 65 tests pass (no regression) |
| `cargo test -p arcane-infra --features rapier-cluster` | 104 tests pass — 68 lib (38 in `rapier_cluster::tests`) + 35 integration + 1 doctest |
| `cargo clippy --all-targets` (both feature configurations) | Silent |
| `cargo fmt --all -- --check` | Clean |
| Doctest in module-level `# Example` | Compiles |
| End-to-end smoke test against running Redis (`arcane-rapier-cluster` binary, ~1000 ticks) | Stable `tick_ms` ~0.07–0.08; WS connection accepted; zero parse failures / broadcast lag |

The 38 `rapier_cluster::tests` cover every documented contract:

- Lifecycle: spawn (first-sight position seeded), despawn (two paths: `pending_removals` + entity-map disappearance), respawn (same UUID), empty entity map.
- Multi-entity: independent advancement of differently-velocitied entities; 500-entity scale test.
- Dynamics: velocity passthrough vs. analytic, gravity vs. kinematic equation, mid-sim velocity change, monotonic velocity growth under gravity.
- User-sim composition: `None` user sim runs pure Rapier; user `on_tick` runs before physics with correct `tick`/`dt`/`game_actions`; user buff modulates velocity; user can request removal via `pending_removals`.
- Determinism / hand-off: same-input → same-output (in-process); state round-trips through despawn / respawn (cluster A → cluster B handoff scenario); contact events do not carry across hand-off.
- V2 contact events + colliders: overlap → Started; distant → no contacts; Cuboid honored; shape change after first-sight is ignored AND `collider_for` called exactly once per entity; one-tick delay; no duplicate Started for persistent overlap.
- Tier-1 contract pinning: Stopped event surfaces on separation; despawn-during-contact does NOT surface Stopped (documented behavior); Ball / Capsule shapes verified directly via `ColliderSet` inspection; multi-substep tick (`dt > FIXED_PHYSICS_DT`); slow-tick accumulator (`dt < FIXED_PHYSICS_DT`); contact resolution applies impulse to partner; `collider_for` invoked freshly on respawn.
- Tier-2 symmetry: V2 ctx propagates `game_actions` / `tick` / `dt`; V2 `pending_removals`; mixed Ball-vs-Cuboid contact; non-default gravity on arbitrary axis; V2 handoff contact-events reset; capsule axis is Y.

## Consequences

### Positive

- Same `node_runner::run_cluster_loop` powers both vanilla and Rapier clusters; networking, replication, neighbor merge, persist are guaranteed identical (literally the same code).
- Vanilla builds remain Rapier-free; existing benchmarks unaffected.
- The wrapper composition pattern is the template for the next backend. The Unreal Cluster Node epic ([`#124`](https://github.com/brainy-bots/arcane/issues/124)) explicitly inherits the composition shape, the entity-keyed in-tick ops convention, and the spawn-time-hook pattern.

### Negative / accepted trade-offs

- **`Mutex<RapierState>` is held during user `on_tick` (V2 path).** Cluster runner is single-threaded per cluster anyway, so contention isn't an issue, but it does mean user code that re-enters Arcane through another path (calling another cluster, scheduling background work) must avoid acquiring the same lock. Documented.
- **Off-spine bodies are explicitly NOT supported.** Game developers wanting "purely visual" debris or particles cannot create un-replicated Rapier bodies. They must use entities (which replicate) or do effects client-side. Trade-off accepted in favor of the wire-format invariant.
- **Cross-cluster joints, multibody articulations, and other advanced Rapier features are not exposed yet.** Tracked in [`#122`](https://github.com/brainy-bots/arcane/issues/122) (gap inventory). Lit up as concrete games need them.
- **`f32` precision for positions** (Rapier 0.32 default). For worlds within ~10⁴ units of origin, sub-millimeter; far-from-origin coordinates lose precision in standard `f32` ways. Documented; switchable to Rapier's `f64` feature in a follow-up if needed.

### Open follow-ups (tracked elsewhere)

- [`#120`](https://github.com/brainy-bots/arcane/issues/120) — Spawn-time hooks (`body_kind_for`, `material_for`, `collision_groups_for`, `is_sensor`).
- [`#121`](https://github.com/brainy-bots/arcane/issues/121) — In-tick imperative ops (impulses, forces, raycasts, joints, teleport).
- [`#122`](https://github.com/brainy-bots/arcane/issues/122) — Gap inventory tracker; lists every Rapier capability with its status.
- [`#119`](https://github.com/brainy-bots/arcane/issues/119) — Terrain epic; the `RapierMapProvider` interface for chunk loading is part of this work.
- [`#124`](https://github.com/brainy-bots/arcane/issues/124) — Unreal Cluster Node epic, applying the lessons codified here.

## References

- [`docs/architecture/physics-backends-and-unreal.md`](../physics-backends-and-unreal.md) — Physics-tick contract, body-kind clarifications.
- [`docs/architecture/entity-model.md`](../entity-model.md) — Unified entity model, per-engine API discipline, terrain handling.
- [`docs/architecture/four-bucket-state-model.md`](../four-bucket-state-model.md) — Spine pose vs replicated user_data vs ephemeral local_data vs SpacetimeDB durable; durable-state-per-entity invariant.
- Memory anchors: `project_unified_entity_model.md`, `project_per_engine_api_pattern.md`, `project_redis_vs_spacetimedb_split.md`, `feedback_refresh_arcane_architecture_before_proposing.md`.
