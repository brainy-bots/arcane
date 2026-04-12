# Four-bucket entity and world state model (v1)

This document is the **canonical reference** for where game data lives in Arcane + SpacetimeDB. Integrators, reviewers, and client authors should align with it.

---

## 1. Why four buckets

Multiplayer backends mix **hot replication** (cluster mesh, clients), **process-local scratch**, and **durable transactional** state. Without named buckets, every design discussion drifts into engine-specific jargon. These four buckets are **coarse** by design: they match what Arcane already implements (`EntityStateEntry`, Redis deltas, optional SpacetimeDB persist) and stay extensible (finer replication can live *inside* bucket 2 later).

---

## 2. The four buckets

| # | Name | What it is | Latency / durability | Typical contents |
|---|------|------------|----------------------|------------------|
| **1** | **Spine (routing + pose)** | Fixed fields the platform understands for routing, replication, and interest management. | Hot path; in-memory; replicated every tick as part of the entity row. | `entity_id`, `cluster_id`, `position`, `velocity` |
| **2** | **Replicated simulation payload** | Game-defined data that **must** cross the cluster mesh and usually reach clients, updated with simulation. | Hot path; serialized on Redis and (today) WebSocket payloads when present. | `EntityStateEntry::user_data` (JSON) |
| **3** | **Cluster-local / ephemeral** | State that **must not** leave this cluster process on the replication wire. | Memory only; **lost** on process crash or if an entry is replaced from a neighbor delta without rehydrating. | `EntityStateEntry::local_data` (JSON); any fields on your [`ClusterSimulation`](../../crates/arcane-core/src/cluster_simulation.rs) implementation |
| **4** | **Durable authoritative** | Transactional, persistent outcomes and tables clients may subscribe to via SpacetimeDB. | Not on the Redis tick hot path; written via reducers / module APIs on a throttled or event-driven cadence. | SpacetimeDB tables, reducers, subscriptions |

---

## 3. Mapping to Rust types and wires

### 3.1 `EntityStateEntry` (`arcane-core`)

Source of truth: [`crates/arcane-core/src/replication_channel.rs`](../../crates/arcane-core/src/replication_channel.rs).

| Field | Bucket | Notes |
|-------|--------|--------|
| `entity_id`, `cluster_id` | **1 — Spine** | Identity and ownership for routing and display. |
| `position`, `velocity` | **1 — Spine** | Pose for simulation and replication. |
| `user_data` | **2 — Replicated simulation** | Omitted from JSON when `null`. On the wire to neighbors and (current reference server) clients. |
| `local_data` | **3 — Cluster-local** | **`#[serde(skip_serializing)]`** — never part of `EntityStateDelta` JSON. Not trusted from clients; set server-side. Incoming neighbor rows arrive with `local_data` defaulted / empty for that entity. |

### 3.2 Cluster simulation hook

[`ClusterSimulation::on_tick`](../../crates/arcane-core/src/cluster_simulation.rs) receives `ClusterTickContext` with mutable `entities: &mut HashMap<Uuid, EntityStateEntry>`.

- May read/write **bucket 1** and **2** (pose and `user_data`) for authoritative simulation.
- May read/write **bucket 3** (`local_data` and internal structs on the simulation type).
- **Bucket 4** is **not** updated inside this hook alone: call SpacetimeDB from your integration layer (reducers, SDK) when discrete events or persist windows require it.

### 3.3 SpacetimeDB

- **Bucket 4** lives in **module tables and reducers** (e.g. inventory, match results, assignments).
- The reference **`spacetimedb_persist`** path snapshots **positions** (and optionally you extend the reducer) from cluster state at a throttled rate — treat that as **hydrating or mirroring** durable views, not as the sole source of high-frequency pose (Redis carries that between clusters).

---

## 4. Trust and validation

| Bucket | Who may write (production target) |
|--------|-----------------------------------|
| 1–2 | **Server / cluster authoritative simulation** after validating client **inputs**. Today’s reference WebSocket accepts `PLAYER_STATE` with pose and `user_data`; a production game should converge on **inputs → server sim** and treat client pose as cheat-prone unless locked down. |
| 3 | **Only** cluster process (simulation, game code). Never accept `local_data` from a client message. |
| 4 | **SpacetimeDB reducers** and controlled server calls; clients subscribe, not mutate tables directly without module rules. |

Document your game’s exact validation policy in your own design doc; this table is the Arcane platform contract.

---

## 5. Neighbor and merge semantics (bucket 3)

When a delta arrives from a neighbor, deserialized `EntityStateEntry` values **do not carry** your previous `local_data` for that entity id if the row is replaced from wire data. Rules of thumb:

- Put **only** recomputable or non-critical state in `local_data`, **or**
- Rehydrate `local_data` after apply using `entity_id` + game rules, **or**
- Keep neighbor entities as kinematic proxies with minimal local state.

---

## 6. Relationship to per-property replication

Engine-style per-property replication (Unreal-style conditions, dormancy, per-field frequency) is **out of scope for v1**. Bucket 2 is intentionally a **single JSON blob** on the hot path; games can version schemas inside `user_data` until finer-grained replication is justified by metrics.

---

## 7. Documentation alignment checklist

Use this when updating types or wires so they stay consistent with this model.

- [ ] Read this doc and [`EntityStateEntry`](../../crates/arcane-core/src/replication_channel.rs) doc comments; confirm no contradiction (fix code comments or this doc in PR if drift).
- [ ] [`docs/architecture/README.md`](README.md) indexes this file.
- [ ] Optional: add a short subsection under [`docs/MODULE_INTERACTIONS.md`](../MODULE_INTERACTIONS.md) pointing here (one paragraph).
- [ ] If benchmarks or demos embed state, add a one-line comment in their README or module pointing to this doc for “where does this field live?”

**Non-goals for this checklist:** unrelated features (per-client visibility, async persist refactor, crash recovery) — track those separately in your own planning.

---

## 8. Quick reference card

```
Spine        → entity_id, cluster_id, position, velocity
Replicated   → user_data (JSON, on Redis + WS in reference server)
Local        → local_data + ClusterSimulation-owned memory
Durable      → SpacetimeDB tables / reducers / subscriptions
```
