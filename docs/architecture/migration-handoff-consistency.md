# Migration handoff consistency — the boundary-tick hazard

| | |
|---|---|
| **Status** | Known hazard + candidate mitigation (not yet a locked decision) |
| **Date** | 2026-09-13 |
| **Applies to** | Entity authority transfer / dynamic migration (deferred in [ADR-002](adr/002-cross-cluster-physics.md); scoped by dynamic tier migration, `#57`) |
| **Relates to** | [clustering-system-requirements.md](clustering-system-requirements.md) §10 (per-migration outcome data, "consistency anomalies if any"), [four-bucket-state-model.md](four-bucket-state-model.md), [ADR-002](adr/002-cross-cluster-physics.md) |

## Summary

Migrating an entity between clusters in Arcane is an **ownership change, not a state
transfer**: the manager decides the new owner and writes ownership to Redis, the
router tells the target cluster it now owns the entity, and Redis enforces
**single-writer per entity** (only the current owner may write that entity's state).
Because the destination cluster is *already* replicating the entity every tick as a
kinematic proxy (pose + `user_data`, see [ADR-002](adr/002-cross-cluster-physics.md)
and [four-bucket-state-model.md](four-bucket-state-model.md)), the flip does not move
state — the state is already there. This is what makes migration cheap.

**The hazard:** cluster ticks are **not synchronized** with each other, nor with the
manager/router. The ownership flip therefore lands at an arbitrary phase relative to
both the old owner's and the new owner's tick cycles. Depending on that phase, the
handoff can lose **0, 1, or 2 ticks** of simulation for the migrating entity, rather
than a fixed, well-defined amount. This is an emergent, nondeterministic outcome of
two (or three) unsynchronized clocks crossing one barrier.

**Scope of the loss:** the rejected write is a **Redis simulation-state** write, so only
buckets 1–2 (pose + transient hot `user_data`) are exposed. Durable discrete outcomes
(kills, pickups, loot, XP) go through SpacetimeDB reducers (bucket 4), a separate
transactional path the flip does not gate — so the hazard is bounded to a tick or two of
transient simulation state on a low-coupling entity, **not** a lost kill. See
"What the dropped tick can and cannot lose" below.

This document records the hazard and a candidate mitigation so it is not rediscovered
the hard way when the authority-transfer / dynamic-migration work (`#57`) is built.

## Why it happens

The manager and router aim to enact an ownership change **faster than a cluster tick**.
The intended, benign outcome is that the old owner's *final* tick is simply lost: after
the flip, the previous owner can no longer write that entity's state to Redis
(single-writer enforcement), so whatever it computed in a tick that was in flight at the
moment of the flip is dropped, and the new owner resumes from the last replicated state.

Because clusters tick independently:

- **Old owner (A):** loses a tick only if the flip lands *after* A began computing tick
  T but *before* A's write for T is accepted. If the flip lands in A's gap between
  writes, A loses nothing (its last tick already committed).
- **New owner (B):** resumes from the newest state it holds. Depending on where B is in
  its own tick cycle when authority arrives, it either re-derives a tick from
  one-tick-stale input (harmless continuity) or steps forward leaving a one-tick hole
  that A also never committed.

Composing the two independent phases:

| Case | When | Ticks lost |
|---|---|---|
| Best | flip lands in A's inter-tick gap (A's last write committed) and B picks up cleanly | **0** |
| Expected | flip lands mid-A-tick; A's in-flight tick is rejected; B resumes at T+1 | **1** |
| Worst | A's tick T rejected *and* B's phase leaves T uncommitted while B's transitional tick started from pre-T state | **2** |

## What the dropped tick can and cannot lose

The migrating entity is, by construction, one the clustering model judged **low-coupling
this instant** (that is *why* it is safe to move — see
[clustering-system-requirements.md](clustering-system-requirements.md) §3). So the entity
is minimally interacting with anyone across the seam at the moment of the flip. The model
does not only decide *where* to cut the interaction graph, it effectively decides *when*
to cut, and it cuts at a low-coupling moment.

**Crucially, the rejected write is a Redis *simulation-state* write.** Redis only carries
buckets 1–2 — spine pose (position/velocity) and transient replicated `user_data` such as
current health/combat state (see [four-bucket-state-model.md](four-bucket-state-model.md)).
**Durable, discrete outcomes — kills, pickups, loot drops, deaths, match results,
inventory, XP — are not on the Redis hot path at all.** They are committed through
SpacetimeDB reducers (bucket 4), a separate transactional path that the ownership flip's
Redis single-writer gate does not touch. SpacetimeDB gives those writes ACID semantics and
its own serialization, so a reducer call the old owner issued for a killing blow on tick T
commits regardless of the Redis ownership flip.

So a dropped boundary tick can only ever lose **simulation state**:

- **Pose (bucket 1):** ~33–100 ms of position/velocity on a low-coupling entity,
  recoverable by extrapolation and within what netcode already tolerates under jitter.
  Harmless.
- **Transient hot combat state (bucket 2):** at most one tick's worth of a value such as a
  health decrement. If the interaction is ongoing it is re-applied by the new owner on the
  next tick; damage that crosses the seam routes to the current authority as an imperative
  op anyway ([ADR-002](adr/002-cross-cluster-physics.md)). Low impact.

The **durable** consequence of a hit (the death, the loot, the XP) is safe because it never
lived in the rejected Redis write — it went to SpacetimeDB. This shrinks the hazard from
"a migration can eat a kill" to "a migration can drop at most a tick of transient
simulation state on a low-coupling entity."

### The one residual case

A durable outcome is at risk **only if the game defers or batches** its SpacetimeDB commit
(e.g. accumulates combat results in bucket-1/2 simulation state and flushes to SpacetimeDB
later) instead of committing it **event-driven on the tick it occurs**. In that case the
not-yet-committed outcome sits in droppable simulation state when the flip lands. The
four-bucket model already directs discrete outcomes to reducers on an event-driven cadence;
following that discipline closes this case (see mitigation 2).

## Candidate mitigation

The fix does **not** require synchronizing cluster clocks (which would be expensive and
against the architecture). Two complementary measures make the seam deterministic:

### 1. A two-phase quiesce/adopt handoff (makes the tick loss deterministic)

Turn the ownership flip from a bare record rewrite into a short sequenced handshake:

1. **Quiesce (old owner A):** manager marks the entity `migrating`. A stops *starting
   new* authoritative ticks for the entity but is allowed to finish and commit any tick
   already in flight (including committing discrete events to the durable path). A
   publishes a final `handoff_tick = T_last` marker.
2. **Adopt (new owner B):** B does not begin authoritative simulation until it has
   ingested A's state through `T_last`, then resumes at `T_last + 1`. The Redis ownership
   record flips between these phases.

Under this protocol A never has an uncommitted computed tick, and B always starts exactly
one tick after A's last committed tick, **regardless of clock phase**. Clock drift then
only affects the *latency* of the handshake (a couple of ticks of wall-clock), not *how
many ticks are lost*. The low-coupling migration condition easily absorbs that latency.

### 2. Keep discrete outcomes on the durable path (preserves the already-safe default)

Durable discrete outcomes are **already safe by default**, because they commit through
SpacetimeDB reducers (bucket 4), not the Redis simulation write the flip gates. This
mitigation just names the invariant that keeps them safe, so a game does not accidentally
regress into the residual case above:

> A discrete outcome must be committed to the durable path (SpacetimeDB reducer, bucket 4)
> **on the tick it occurs** — never accumulated in bucket-1/2 simulation state and flushed
> to SpacetimeDB later.

Committing event-driven (the cadence the [four-bucket model](four-bucket-state-model.md)
already prescribes) guarantees that a rejected boundary-tick Redis write can only ever
carry transient simulation state (pose, hot `user_data`), which is the harmless case. A
game that instead batches durable results into replicated simulation state reintroduces
the risk that a batch pending at the flip is dropped; the invariant forbids that pattern.

With both measures, the only thing a migration can lose is pose advancement on a
low-coupling entity, and the amount is deterministic rather than phase-dependent.

## Current state (what is implemented vs designed)

- **Implemented:** frame-by-frame replication of pose + `user_data` to neighbors as
  kinematic proxies (ADR-002); single-writer-per-entity ownership in Redis; manager
  decides ownership and the router enacts it. This is what makes the state already-present
  at the destination.
- **Designed / deferred:** atomic authority transfer itself (ADR-002 Layer 3), including
  proxy→dynamic promotion on the destination and the handoff sequencing described here.
  Tracked with dynamic tier migration (`#57`).
- **Open follow-up:** per-migration outcome telemetry (migration latency, player-perceived
  seam duration, consistency anomalies) is already called out as needed in
  [clustering-system-requirements.md](clustering-system-requirements.md) §10; it is what
  would let us *measure* whether the boundary-tick hazard ever manifests in practice, and
  turn migration cost/quality into a model input.

## Decision status

This is a **recorded hazard with a candidate mitigation**, not a locked decision. The
quiesce/adopt handshake and the commit-before-release invariant should be evaluated and,
if adopted, promoted to an ADR when the authority-transfer / dynamic-migration work
(`#57`) is designed and implemented.
