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
| **3** | **Cluster-local / ephemeral** | State that **must not** leave this cluster process on the replication wire. | Memory only; **lost** on process crash or if an entry is replaced from a neighbor delta without rehydrating. | `EntityStateEntry::local_data` (JSON); plus any extra process-only state your simulation holds (see [physics-backends-and-unreal.md](physics-backends-and-unreal.md) for optional per-tick hooks) |
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
| `local_data` | **3 — Cluster-local** | **`skip_serializing` + `skip_deserializing`:** not on the wire; JSON must not hydrate bucket 3 from neighbors/Redis. Set only in-process. |

### 3.2 Per-tick simulation (optional)

Authoritative simulation may update **buckets 1–3** in memory each tick (pose, `user_data`, `local_data`). A future or game-specific **per-tick hook** in the cluster runtime is described in [physics-backends-and-unreal.md](physics-backends-and-unreal.md) (not required for using the four-bucket **data** model on the wire).

- Typical rules: read/write **bucket 1** and **2** for integrated motion and replicated game fields; use **bucket 3** for scratch that must not leave the process on Redis.
- **Bucket 4** is **not** satisfied by mutating `EntityStateEntry` alone: use SpacetimeDB reducers / module APIs when discrete events or durable tables require it.

### 3.3 SpacetimeDB

- **Bucket 4** lives in **module tables and reducers** (e.g. inventory, match results, assignments).
- The reference **`spacetimedb_persist`** path snapshots **positions** (and optionally you extend the reducer) from cluster state at a throttled rate — treat that as **hydrating or mirroring** durable views, not as the sole source of high-frequency pose (Redis carries that between clusters).

---

## 4. Trust and validation

| Bucket | Who may write (production target) |
|--------|-----------------------------------|
| 1–2 | **Server / cluster authoritative simulation** after validating client **inputs**. Today’s reference WebSocket accepts `PLAYER_STATE` with pose and `user_data`; a production game should converge on **inputs → server sim** and treat client pose as cheat-prone unless locked down. |
| 3 | **Only** cluster process (simulation, game code). Never accept `local_data` from a client message; replication JSON cannot populate it (`skip_deserializing` on the type). |
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

- [x] Read this doc and [`EntityStateEntry`](../../crates/arcane-core/src/replication_channel.rs) doc comments; confirm no contradiction (fix code comments or this doc in PR if drift).
- [x] [`docs/architecture/README.md`](README.md) indexes this file.
- [x] Pointer from [`docs/MODULE_INTERACTIONS.md`](../MODULE_INTERACTIONS.md).
- [ ] If external benchmarks or demos embed state, pin **crate + module versions** and record manifests next to results (see §8).

**Non-goals for this checklist:** unrelated features (per-client visibility, async persist refactor, crash recovery) — track those separately in your own planning.

---

## 8. Workload and version pinning (benchmarks)

Ceilings and comparisons are only meaningful when **Arcane’s crate version**, any **SpacetimeDB module version**, and **run manifests** (rates, visibility, authority path) are recorded next to published numbers. External harness repositories should store that metadata beside CSVs or dashboards so “what lived in which bucket” can be reconstructed.

---

## 9. Quick reference card

```
Spine        → entity_id, cluster_id, position, velocity
Replicated   → user_data (JSON, on Redis + WS in reference server)
Local        → local_data + other process-only simulation memory
Durable      → SpacetimeDB tables / reducers / subscriptions
```
