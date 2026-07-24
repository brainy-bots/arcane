# Progressive API design — an Arcane pillar

> **Default is free and works. Optimization is opt-in. Complexity paid ≈ optimization gained.**

Arcane is built for game teams that span from one person prototyping in a weekend to a studio running a live title at scale. Those teams sit at very different points on a complexity budget. A framework that makes both first-class requires deliberate **progressive disclosure** — every platform capability exposes a ladder the developer can climb as their needs grow, without rewriting what already works.

This document is the platform-level contract: when you add or extend an Arcane capability, it must fit the ladder shape below.

---

## 1. The ladder shape

| Level | Typical cost | What it offers | When you need it |
|-------|--------------|----------------|------------------|
| **0 — Default** | No code. No choice. | Sensible behavior out of the box. | Always. Every Arcane game starts here. |
| **1 — Data-level** | Put data in a field the platform already knows about. | Auto-handling extends to cover it. | Most games live here. |
| **2 — Simple knob** | One method call or one config value. | Tune cadence, trigger immediately, opt out. | Specific gameplay moments (death, match-end, irreversible events). |
| **3 — Typed opt-in** | Implement a trait method or register a custom reducer. | Typed schemas, multi-cadence, custom payload shape. | Games that need SQL-queryable state or segregated concerns. |
| **4 — Escape hatch** | Raw use of the underlying system (SpacetimeDB SDK, Redis client, etc.). | Unlimited — but you own the complexity. | Rare. Custom validation, anti-cheat, novel cross-system flows. |

Each level is **additive**. A developer at level 1 does not rewrite to reach level 2 — they *add* the method call to the one action that needs it. A developer at level 3 still benefits from level-0/1 defaults for everything they haven't opted out of.

---

## 2. Illustrated: persistence

The clearest worked example is cluster → SpacetimeDB durable state.

| Level | Example | Code required |
|-------|---------|---------------|
| **0** | Entity positions auto-persist at 1 Hz via `set_entities`. | None. |
| **1** | Any mutable per-entity state (HP, inventory, buffs, match-local) goes into `EntityStateEntry::user_data`. The auto-persist path extends to carry it. | Mutate `entity.user_data` in `on_tick`. |
| **2** | Critical event needs to persist *this tick*, not at the next 1 Hz flush. | `ctx.request_persist_now()` in `on_tick`. |
| **3** | Game needs a typed `Inventory` table SpacetimeDB clients can subscribe to with SQL queries. | Implement `ClusterSimulation::snapshot()` returning `Snapshot { reducer: "persist_inventory", payload: ... }`. |
| **4** | Rare authoritative transaction requires sync reducer call (shop purchase, match-end with payouts). | `spacetime.call("complete_purchase", args).await`. |

A small game lives at level 1 for its entire lifetime. A medium game adds level-2 flushes for 3-5 critical events. A large game might add a level-3 snapshot for inventory and a level-4 reducer for the in-game shop — and leave everything else at level 1.

**Crucially: a team never has to decide on all five levels up front.** They start at 0/1 and climb only where profiling or player experience demands it.

---

## 2.1 Persistence and session lifecycle ladder (L0–L3)

Entity persistence and session lifecycle management follow the same ladder shape as the broader persistence example above:

| Level | Session lifecycle | Env surface | Recovery guarantee | Typical use |
|-------|-------------------|-------------|------------------|---|
| **L0** | Ephemeral — entity exists only while cluster is active; no recovery on crash. | `ARCANE_PERSISTENCE=ephemeral` (default) | None. Entity loss on crash. | Prototypes, session-only games, ammunition/visual effects |
| **L1** | Short-term reconnect — entity parks in Redis with TTL; reconnect within window. `ARCANE_RECONNECT_TTL_SECS=300` (default 5m). | `ARCANE_PERSISTENCE=short-term` + `ARCANE_RECONNECT_TTL_SECS=<seconds>` + `NODE_CLIENT_IDLE_TIMEOUT_SECS=<seconds>` | Reconnect to same entity within TTL; loss after expiry. | Most games during development and live ops; player sessions, temporary items |
| **L2** | Full durable — entity persists in SpacetimeDB; survives full cluster restart. | `ARCANE_PERSISTENCE=full` | Durable recovery across any cluster crash or graceful restart. | Live games, persistent player data, valuable items |
| **L3** | Game-defined — game extends bucket 4 with custom persistence logic and triggers. | `ARCANE_PERSISTENCE=full` + custom reducer config | Custom recovery rules per entity type. | Games with genre-specific state (quest progress, faction rank, custom transactions) |

All levels implement the session-lifecycle invariant: every entity has defined connect/disconnect/reconnect/leave paths (see [`four-bucket-state-model.md`](four-bucket-state-model.md)).

Environment variables:
- **`ARCANE_PERSISTENCE`**: `ephemeral` (L0, default) | `short-term` (L1) | `full` (L2+). Controls whether durable SpacetimeDB storage is enabled.
- **`ARCANE_RECONNECT_TTL_SECS`**: How long a disconnected entity parks in Redis before expiry (L1). Default 300 (5 minutes). Ignored at L0 and L2+.
- **`NODE_CLIENT_IDLE_TIMEOUT_SECS`**: How long a client can be idle before the server considers the session dead (L1 reconnection window closes, L2+ session ends). Default per cluster type. Interacts with reconnection TTL: a client idle longer than this window cannot reconnect, even within TTL.

Note: `SPACETIMEDB_PERSIST=1` (pre-2026 env var) is honored for backwards compatibility, equivalent to `ARCANE_PERSISTENCE=full`.

---

## 3. Other platform areas (ladder sketches)

The same shape applies across the library. For each area, the current platform offers level 0/1 and leaves level 2+ as deferred work triggered by real use cases.

### 3.1 Replication (cluster → neighbor clusters)

| Level | Example |
|-------|---------|
| 0 | Entity spine (`position`, `velocity`) auto-replicates to neighbors at tick rate. |
| 1 | `user_data` auto-replicates as part of the same delta payload. |
| 2 | Per-field dormancy / relevance filters (only send to clusters that need it). |
| 3 | Custom replication channels (e.g. rare cross-realm broadcasts). |
| 4 | Direct Redis pub/sub. |

### 3.2 Cross-cluster game events (e.g. attacker and target on different clusters)

| Level | Example |
|-------|---------|
| 0 | (to be designed) — most games avoid this by colocating interacting entities. |
| 1 | Simple "route this event to the cluster owning entity X" helper. |
| 2 | Retry + timeout policies for the helper. |
| 3 | Typed protocol for complex cross-cluster interactions. |
| 4 | Direct Redis-based inter-cluster messaging. |

### 3.3 Observability

| Level | Example |
|-------|---------|
| 0 | `/stats` endpoint on every cluster — `ws_accepts`, `entities_current`, tick timings, parse failures. |
| 1 | Structured log lines for key transitions. |
| 2 | `ctx.emit_counter("combat_events_per_tick", n)` developer-visible counters. |
| 3 | Custom metrics sinks (Prometheus, custom exporter). |
| 4 | Direct integration with an observability backend. |

### 3.4 Authority model

| Level | Example |
|-------|---------|
| 0 | Cluster-local simulation decides movement, physics, damage resolution. |
| 1 | Deterministic game rules declared in the simulation; cluster is the authority for its entities. |
| 2 | Opt-in SpacetimeDB reducer validation for specific actions (trade, shop). |
| 3 | Custom validation pipeline (anti-cheat, server-side replay). |
| 4 | Escape hatch: direct SpacetimeDB reducer calls with arbitrary logic. |

---

## 4. Rules for contributors

When you add or extend a platform capability, follow these:

1. **Always provide level 0 and level 1.** If your PR only adds a level-3 method with no default, push back and redesign. There is almost always a reasonable default that covers 80% of cases for free.
2. **Do not build level-2+ speculatively.** Every level 2+ API must have a documented acceptance trigger — a real game or concrete scenario that can't be handled by existing levels. File it as an issue, wait for the trigger.
3. **Levels are additive, not replacing.** Your new level-3 API must not break or penalize level-0/1 users. If it does, the abstraction is wrong.
4. **Name the level in PR descriptions.** "This PR adds a level-2 knob for persist cadence" is clear; "this PR adds cadence control" leaves reviewers guessing whether it's replacing something.
5. **Document the trigger in the ladder table.** When a level 2+ feature ships, update the relevant ladder in this doc (or the feature's own doc) with the triggering use case — future contributors see why it exists.

### 4.1 Recognizing drift

Push back on platform PRs that:

- Add a new trait method with no matching level-0/1 default.
- Add flexibility "for future use" with no concrete trigger.
- Force existing level-0 users to learn the new API.
- Bundle unrelated level-3 capabilities into one PR because they happen to touch the same file.

### 4.2 Documenting deferred levels

For levels 2+ that haven't been built yet, file a tracking issue with this structure:

```
Title: [ladder-L<n>] <area>: <capability>

Current situation (level below):
- What works today, what's the workaround, what's the pain.

Trigger for building this level:
- First real game or use case that can't be handled by the current ladder.
- Example: "Game X needs the combat log as SQL-queryable table; user_data JSON
  blob doesn't let them query by timestamp."

Proposed shape (non-binding):
- Rough API sketch. Not locked — real trigger informs the final design.

Level above (if any):
- What the level-(n+1) escape hatch looks like and why we're not just documenting that.
```

---

## 5. Relationship to other architecture docs

- [`four-bucket-state-model.md`](four-bucket-state-model.md) — defines where data lives. The buckets map naturally to levels 0–1: spine auto-persists (L0), `user_data` auto-persists when extended (L1), `local_data` is process-only, SpacetimeDB is the durable target the levels climb toward.
- [`connection-types.md`](connection-types.md) — catalogues the wire-level paths. Levels 2+ for persistence, replication, and cross-cluster build on these.
- [`component-index.md`](component-index.md) — lists every platform component. New components should state which ladder level they sit at and what ladder they participate in.

---

## 6. Quick reference

```
Default (L0)   → free, no code, works for everyone
Data-level (L1) → put data in known fields, auto-handling covers it
Simple knob (L2) → one method / config, specific use cases
Typed opt-in (L3) → trait method or custom reducer, real-world triggered
Escape hatch (L4) → raw underlying system, rare, always available
```

When designing: start at L0, prove it's enough, add higher levels *only* when a concrete case forces it. The pillar holds so long as a new contributor can ship their first game at L0/L1 without knowing anything about L2+.
