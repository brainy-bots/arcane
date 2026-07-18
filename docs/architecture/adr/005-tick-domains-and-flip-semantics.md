# ADR-005 — Tick domains, flip application semantics, and gate confirmation

| | |
|---|---|
| **Status** | Accepted (founder-approved v1 decisions, 2026-07-18) |
| **Context** | Epic #271 (production binary wiring); design doc §8; `resolve_authoritative` (#207); `ReplicationGate` (#215, wired by #276) |
| **Decides** | What `effective_tick` means across independently-ticking processes; when a node applies a received flip; what "confirmed replication" means in v1 |

## Problem

The library-level control loop was proven in one process with one tick counter. In production there are **three independent tick domains**:

1. **Manager cycle counter** — increments per `ManagerRuntime::run_cycle` (decision cadence, ~1s).
2. **Node tick counters** — one per node process, incrementing at simulation rate (~30Hz), mutually unsynchronized.
3. **Wall clock** — nobody's authority.

`OwnershipFlip.effective_tick` is stamped from the **manager's** counter. The exactly-once XOR proof (`resolve_authoritative(entity, cluster, map, tick, flip)`) evaluates flips against a tick argument — but *whose* tick? In-process tests could conflate them; real processes cannot.

## Decision 1 — flips apply at the receiving node's next frame boundary

A node applies an inbox frame's ownership entries (`apply_inbox_frame`) during `drain_inputs`, i.e. **before the next simulation step it runs after receipt**. From that step on, `submit_entities`' ownership gate reflects the new owner. There is no attempt to coordinate a simultaneous cut-over instant across nodes.

**Why this is safe:** the §8 two-step protocol makes precise synchronization unnecessary.
- The **losing** node keeps writing until it applies the flip; the **gaining** node starts writing when *it* applies the flip. Redis pub/sub delivers the same frame to both within jitter of each other; during the (typically sub-tick) window where both or neither have applied it, the existing **local-wins merge dedup** (`merge_with_neighbor_latest`) and last-writer-wins on the replication channel absorb the overlap — the same mechanism that already covers the one-tick overlap inside a single domain (§8 step 2).
- Both maps converge to the same owner as soon as both have drained the frame. No state is lost either way: the gaining node has been replicating the entity for ≥N manager cycles (Decision 3), so its copy is current within one publish interval.

## Decision 2 — `effective_tick` is ordering metadata, not a synchronization point

`effective_tick` remains manager-domain. Consumers use it for:
- **Ordering/dedup** among multiple flips for the same entity (a later manager decision supersedes an earlier one).
- **Diagnostics** (correlating a flip with the manager cycle that produced it).

Consumers must NOT compare it against a node-local tick counter to decide *when* to apply (Decision 1 governs that). `resolve_authoritative`'s tick argument is the **caller's own domain** and is meaningful only for reasoning within that domain; cross-domain XOR assertions are made *per node map after frame application*, which is what the E2E suites assert.

**Rejected alternative:** a shared tick source (Redis INCR or manager-published epoch). Adds a hot-path dependency and coupling for no correctness gain — the two-step protocol already tolerates application skew. Revisit only if a future mechanism genuinely needs a global order (e.g. cross-cluster physics constraint handoff).

## Decision 3 — v1 gate confirmation = manager-side delivery counting

`ReplicationGate` counts **frames the manager published containing the entity to the destination's inbox** (N consecutive cycles, default 3). It does not confirm the destination *processed* them.

**Honest limitation:** a destination node that is subscribed but stalled (or has a full inbox) can be granted ownership it never saw arrive. v1 accepts this because (a) the inbox subscription is verified at node startup, (b) a stalled node fails visibly and quickly at other layers (its own state key stops updating — the manager can observe `ClusterStateDoc.tick` staleness and refuse flips to stale destinations, which B3 implements as a cheap guard), and (c) true acknowledgment needs a node→manager return channel.

**Follow-up (not v1):** node-side acks — the node reports last-applied inbox tick in its `ClusterStateDoc`; the gate confirms on acked ticks instead of published frames. The state-key channel already carries the field naturally; no new transport needed. Tracked as a B-series follow-up issue when measurement shows the v1 guard insufficient.

## Consequences

- `apply_inbox_frame` stays as merged (applies on receipt) — no change.
- B3's manager binary adds the staleness guard: no flip publishes to a destination whose state key is older than `MANAGER_STALE_LIMIT` (default: 3 × cadence).
- The multi-process E2E (B5) asserts exactly-once **per node map** (each node's own OwnershipMap never has the entity owned by both endpoints after both have drained), not against a fictional global tick.
- ADR-004's runtime-assurance framing extends here: the deterministic two-step protocol + merge dedup is the safety envelope; timing precision is an optimization, never a correctness dependency.
