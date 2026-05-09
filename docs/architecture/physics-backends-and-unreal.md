# Physics backends: Unreal (Chaos) first, multi-engine path

This document describes how to integrate **authoritative physics** with Arcane: tick placement, contracts, and a recommended Unreal (Chaos) layout. It complements [four-bucket-state-model.md](four-bucket-state-model.md) and the linked Rust sources. An **implementation checklist** appears at the end.

---

## 1. Goal

- Run **authoritative physics** in the cluster tick path, with **Unreal Engine 5 Chaos** as the **first** concrete backend.
- Keep **arcane-core** and **arcane-infra** free of Unreal/Chaos/Rapier dependencies.
- Leave a **clear path** for a second backend (e.g. Rapier in Rust) by sharing the same integration contract.

---

## 2. Non-goals (v1)

- Loading Chaos inside `arcane-cluster` via FFI from Rust (possible later; high complexity).
- Replacing `EntityStateEntry` with a binary ECS snapshot (future optimization).
- Solving cross-shard physics handoff in full generality (document assumptions; implement minimal v1).

---

## 3. Where physics plugs in (Rust reference server)

Read in order:

1. [`crates/arcane-infra/src/cluster_runner.rs`](../../crates/arcane-infra/src/cluster_runner.rs) — `run_cluster_loop`: drain client updates → optional injected entities → neighbor deltas → **`simulate_before_tick`** → **`tick`** → persist → WebSocket send.
2. [`crates/arcane-infra/src/cluster_server.rs`](../../crates/arcane-infra/src/cluster_server.rs) — `simulate_before_tick` calls [`ClusterSimulation::on_tick`](../../crates/arcane-core/src/cluster_simulation.rs).
3. [`crates/arcane-core/src/cluster_simulation.rs`](../../crates/arcane-core/src/cluster_simulation.rs) — trait + `ClusterTickContext` (`entities`, `dt_seconds`, `tick`, `pending_removals`).

**Contract today:** implement `ClusterSimulation: Send + Sync` with `fn on_tick(&self, ctx: &mut ClusterTickContext<'_>)`. The trait uses **`&self`**, so physics world state must use **interior mutability** (e.g. `Mutex<ChaosWorldHandle>` or solver’s internal sync).

**After `on_tick`:** `ClusterServer::tick` builds `EntityStateDelta` and replication runs. Pose must be written back to `EntityStateEntry::position` / `velocity` (and `user_data` if needed) before `tick` returns.

---

## 4. Recommended default integration shape (v1 demo)

Three integration shapes are possible (Rust cluster + UE sidecar, Unreal-native, Rust hosting a UE DLL). **Default for the first shippable Unreal demo:**

### **Unreal-native cluster (preferred v1)**

- **One process:** Unreal **Dedicated Server** build.
- **Chaos** runs entirely inside UE; no requirement for a separate Rust `arcane-cluster` process for physics stepping.
- **Networking:** Implement WebSocket client (or reuse **arcane-client-unreal** patterns if applicable) to speak the same **JSON** shapes the reference server expects for **inbound** player updates and **outbound** state deltas, **or** document a thin translation layer if the plugin uses a different framing.
- **Mapping:** Game code implements the equivalent of “each tick: apply inputs already reflected in entity map → step Chaos → write poses to the structures that back replication to Arcane wire format.”

**Rationale:** Chaos stays in the engine’s supported environment; no mixed Rust/UE DLL lifetime issues for v1.

### When to spike the alternative (Rust cluster + Unreal sidecar)

Choose only if Unreal-native WebSocket + replication proves infeasible for your team or if you must keep the existing `arcane-cluster` binary as the only publisher to Redis. Document the decision in a short ADR in `docs/architecture/adr/`.

---

## 5. Unreal / Chaos specifics

- **UE5 authoritative physics:** use **Chaos** rigid bodies / scene queries as appropriate for your game. PhysX legacy paths exist; **do not** anchor v1 on deprecated paths without an explicit reason.
- **API surface:** Chaos APIs evolve by UE minor version — **pin the engine version** in the demo README and verify compile steps on that version.
- **Tick alignment:** Arcane reference loop uses **20 Hz** (`TICK_RATE_HZ` in `cluster_runner.rs`). Physics often wants **fixed sub-steps** (e.g. 60 Hz): run `n` Chaos steps inside one `on_tick` (or one UE frame) using `ctx.dt_seconds` or a fixed `1/60` substep until accumulated time is consumed. Document chosen policy in the demo.

---

## 6. Entity ↔ body mapping (implementer spec)

Maintain a **bidirectional map** in the integration layer:

| Concept | Responsibility |
|---------|----------------|
| `entity_id` (`Uuid`) | Stable Arcane / wire identity; store as FGuid or string in UE per project convention. |
| Chaos / Rapier actor / body | Spawn when a new owned entity appears in the authoritative set; destroy when removed or when `pending_removals`-style lifecycle fires. |
| Neighbor entities | Appear as **kinematic proxy bodies** in the local Rapier world. Raycasts and collision detection work against them. Write ops on proxies route via `arcane:physics_events:<target_cluster_uuid>` Redis channel to the authority cluster. Contact events flow back bidirectionally. See [ADR-002](adr/002-cross-cluster-physics.md) for full design. Authority transfer (entity migration) deferred to affinity clustering infrastructure. |
| `user_data` | Optional: stiffness, hitbox id, team — replicated; keep small. |
| `local_data` | Solver scratch, cooldowns — **not** on Redis wire; see [four-bucket-state-model.md](four-bucket-state-model.md). |
| **Body kind** | Per-entity, declared at first-sight via `body_kind_for` hook. `Dynamic` (players, projectiles, debris), `KinematicPositionBased` / `KinematicVelocityBased` (server-controlled motion), `Fixed` (walls, placed structures). Default `Dynamic`. See [entity-model.md §4](entity-model.md). |
| **Terrain / world geometry** | **Not entities.** Loaded into the cluster's physics world automatically by the Arcane runtime based on entity positions. Game developers do not insert terrain colliders by hand. See [issue #119](https://github.com/brainy-bots/arcane/issues/119). |

**Spawn sync:** On first sight of `entity_id`, create the appropriate body via the `body_kind_for` + `collider_for` hooks. Default body kind is `Dynamic` with a sphere collider matching `RapierConfig::default_body_radius`.

**Despawn:** On removal from cluster authority or `pending_removals`, destroy physics objects and clear handles.

**Sleeping bodies:** stationary Dynamic / Kinematic bodies and all Fixed bodies are essentially free per tick — Rapier's sleep mechanism + Fixed-body solver-skip means a cluster with hundreds of stationary entities pays cost proportional only to active (awake) bodies. The "no entities → no simulation" intuition is preserved by this mechanism without needing a separate concept for stationary objects.

---

## 7. Multi-backend path (second engine)

The Rapier (Rust) backend has landed and is documented in [ADR-001](adr/001-rapier-cluster-integration-shape.md) — composition over inheritance, in-process Rust, single Cargo feature flag, no separate crate. The decisions are captured there:

- **No new `PhysicsBackend` trait.** `RapierClusterSim` is itself a `ClusterSimulation` impl that wraps a user `ClusterSimulation` (or, in the V2 path, a sibling `RapierClusterSimulation`).
- **Selection** is build-time (Cargo features) and construction-time (which `Arc<dyn ClusterSimulation>` is passed to `run_cluster_loop`); no runtime plugin registry.
- **Rapier as `optional = true` Cargo dep on `arcane-infra` behind feature `rapier-cluster`.** Vanilla builds pull zero `rapier3d`. No separate crate needed; the feature-flag pattern is sufficient.
- **Per-engine API discipline:** Rapier-specific types (`RapierColliderShape`, `RapierBodyKind`, `RapierMaterial`) stay in `arcane-infra::rapier_cluster`. They are **not** promoted to engine-neutral `arcane-core` types — see [`entity-model.md` §8](entity-model.md) for why.

Cross-cluster physics (neighbor entities as kinematic proxies + imperative-op routing) is documented in [ADR-002](adr/002-cross-cluster-physics.md).

The Unreal/Chaos backend will follow the same composition pattern but with engine-native concerns (UE-native types, World Partition integration, Y↔Z axis swap, ×100 unit scale at the wire boundary). [`#124`](https://github.com/brainy-bots/arcane/issues/124) is the implementation epic; ADR-003 (pending) will capture the Unreal-side decisions.

---

## 8. JSON and wire compatibility (Rust reference)

Reference server parses **PLAYER_STATE** in [`ws_server.rs`](../../crates/arcane-infra/src/ws_server.rs). Outbound messages are serialized **`EntityStateDelta`**. The Unreal demo must **match** these contracts or include a documented adapter.

Minimum fields for an entry (see `EntityStateEntry`): `entity_id`, `cluster_id`, `position`, `velocity`, optional `user_data`.

---

## 9. Testing expectations

| Test | Purpose |
|------|--------|
| Unit | Mock `ClusterTickContext` with a few `EntityStateEntry` rows; assert after `on_tick` positions change predictably for known forces. |
| Integration | Dedicated server headless: two actors, one tick, deterministic Chaos step (where UE allows). |
| Soak | Optional: run against local Redis + second cluster process if using Rust replication path. |

---

## 10. Deliverables (what “done” looks like)

1. **ADR** (one page) in `docs/architecture/adr/` naming the chosen integration shape (default: Unreal-native) and UE version pin.
2. **Demo project** or module path listed in the ADR (repo name, branch, how to build UDS).
3. **Chaos step** wired so authoritative poses match **bucket 1** (`position`/`velocity`) per [four-bucket-state-model.md](four-bucket-state-model.md).
4. **README** in the demo repo: build, run, how it connects to Manager/cluster if applicable.
5. **Follow-up issue** filed for second backend stub (optional) or visibility filtering (#4) if the demo exposes wallhack risk.

---

## 11. Implementation checklist

**Phase A — Read and freeze decisions**

- [ ] Read `cluster_runner.rs`, `cluster_server.rs`, `cluster_simulation.rs`, `replication_channel.rs`.
- [ ] Read [four-bucket-state-model.md](four-bucket-state-model.md).
- [ ] Write ADR: integration shape + UE version + tick/substep policy.

**Phase B — Unreal (Chaos)**

- [ ] Create or extend UE **dedicated server** target; enable Chaos physics.
- [ ] Implement entity_id ↔ actor/body map; spawn/despawn rules.
- [ ] Per-frame (or fixed tick): apply server-authoritative inputs, step Chaos (with sub-stepping if needed), write back pose to wire structures.
- [ ] Connect to cluster WebSocket / Redis per chosen architecture; validate JSON against reference `EntityStateDelta`.

**Phase C — Hardening**

- [ ] Document trust: client vs server authority for movement (align with four-bucket trust table).
- [ ] Add tests listed in §9; document how to run them in CI or locally.

**Phase D — Optional**

- [ ] Stub `ClusterSimulation` implementation in Rust with Rapier for CI compile smoke (separate crate).
- [ ] If the demo lives in another repo, cross-link it from this repo’s `docs/architecture/adr/` or the demo’s own README.

---

## 12. File index (quick open)

| File | Role |
|------|------|
| `arcane-core/src/cluster_simulation.rs` | Trait to implement (Rust backend) or mirror semantically (UE). |
| `arcane-core/src/replication_channel.rs` | `EntityStateEntry`, delta shape. |
| `arcane-infra/src/cluster_runner.rs` | Tick order and `dt_seconds`. |
| `arcane-infra/src/ws_server.rs` | Inbound `PLAYER_STATE` JSON. |
| `arcane-infra/src/bin/arcane_cluster.rs` | Today passes `None` for simulation — Rust games pass `Some(...)`. |
