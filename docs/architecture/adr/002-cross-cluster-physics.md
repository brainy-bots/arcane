# ADR-002: Cross-cluster physics interaction — kinematic proxies + imperative-op routing

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-05-09 |
| **Implemented in** | PR [`brainy-bots/arcane#128`](https://github.com/brainy-bots/arcane/pull/128) (epic PR); sub-PRs [`#130`](https://github.com/brainy-bots/arcane/pull/130), [`#131`](https://github.com/brainy-bots/arcane/pull/131), [`#132`](https://github.com/brainy-bots/arcane/pull/132) |
| **Related epic** | [`#127`](https://github.com/brainy-bots/arcane/issues/127) — Cross-cluster physics interaction |
| **Related issues** | [`#117`](https://github.com/brainy-bots/arcane/issues/117), [`#120`](https://github.com/brainy-bots/arcane/issues/120), [`#121`](https://github.com/brainy-bots/arcane/issues/121), [`#122`](https://github.com/brainy-bots/arcane/issues/122) |

## Context

PR #128 delivered single-cluster Rapier physics (documented in ADR-001). Cross-cluster physics is an edge case — affinity clustering actively prevents it by co-locating interacting entities. But it must work for capacity overflow, prediction misses, and relaxed-grouping scenarios (deterministic projectiles).

The decision space had three dimensions:

1. **Proxy representation** — should neighbor entities appear as physics bodies in the local Rapier world, or should we defer all cross-cluster interaction?
2. **Authority and operations** — when a local entity interacts with a neighbor, who owns the result? Does the local cluster apply force, or does it just read state?
3. **Phased rollout** — implement all three layers (proxies, imperative ops, authority transfer) at once, or stage them?

## Decision

Implement **Layers 1 and 2** of the three-layer design from issue #127. Layer 3 (authority transfer) defers until entity migration infrastructure exists.

### Layer 1: Kinematic proxies

**Neighbor entities spawn as `KinematicPositionBased` bodies in the local Rapier world.** They participate in raycasts and collision detection but do not move under physics simulation. Their position is synced every tick from the neighbor cluster's replication delta.

- Proxies are created and removed in lockstep with the entity's presence in the local `neighbor_entities` map (populated by `merge_neighbor_updates`).
- Proxy position follows the replicated `entity.position` each tick — no integration step on the proxy itself.
- Raycasts against proxies work; they report collisions as if the neighbor entity's collision bounds were present.
- Contact events involving proxies are stored and surfaced to the game logic the same way as same-cluster contacts (with one-tick delay).

### Layer 2: Imperative-op routing

**Write ops on proxies route via Redis to the authority cluster.** When a `PhysicsHandle` method targets a proxy (e.g., `apply_impulse(neighbor_id, force)`), the operation is JSON-encoded and published to `arcane:physics_events:<target_cluster_uuid>`.

- The authority cluster receives the event, applies the operation to its local copy of the entity, and the result (velocity change, damage, etc.) propagates back through the next replication delta.
- Operations are **fire-and-forget V1** — same reliability model as entity replication (Redis persistence, no ACK).
- Inbound ops from other clusters are applied at the start of `run_physics_phase`, after `on_tick` runs but before Rapier steps, ensuring intent is visible to the authoritative physics.
- **Contact events flow back:** when a proxy participates in a collision, the contact event is stored locally (for the local cluster's game logic) and also published back to the proxy's authority cluster. Both sides see the collision.

### Layer 3 (deferred): Authority transfer

Entity migration between clusters (the proxy becomes real, the real becomes a proxy) requires:
- Atomic ownership transfer across cluster boundaries.
- Joint and contact-state coherence during handoff.
- Deterministic state snapshot semantics.

This ships with the **affinity clustering infrastructure epic** ([`#135`](https://github.com/brainy-bots/arcane/issues/135) — planned). Until then, entities stay on their original cluster.

### Key decisions

- **JSON encoding for physics events** — consistent with entity replication; events are `{entity_id, op_type, op_data}` tuples.
- **Proxies always `KinematicPositionBased`** — kinematic position-based is the cheapest mode; configurable per-entity proxy kind deferred until a game requests it.
- **Fire-and-forget V1 reliability** — same as entity replication. A follow-up (when Redis durability proves insufficient for gameplay) will add per-op acks and replay semantics.
- **Entity_id dedup at merge time** — when merging neighbor deltas, local entities take precedence if both local and neighbor have the same `entity_id`. This shouldn't happen in correctly-implemented affinity, but the rule is deterministic.
- **Cross-cluster joints return `None`** — `PhysicsHandle::create_joint` returns `None` if either entity is a proxy. Affinity must co-locate joint participants. A future layer (deferred with authority transfer) will lift this.
- **Per-cluster Redis topic** — ops route to `arcane:physics_events:<target_cluster_uuid>`. The target cluster consumes its own topic, applies ops, and publishes contact events back to the originating cluster.

## Alternatives considered

### A. No proxies — neighbor entities invisible to physics

Rejected. This breaks raycast and intersection queries when entities span cluster boundaries. Scenarios like deterministic projectiles (fired client-side, validated server-side) need cross-cluster raycasts. Manual workarounds (replicate neighbor entities as a separate data structure) duplicate entity replication.

### B. Full authority-transfer at Layer 3 on day one

Rejected. Authority transfer is complex: entity migration mid-tick, joint coherence, atomic state snapshot. Affinity clustering (which prevents most cross-cluster interactions) ships first; authority transfer ships as a separate epic. The two-layer design is stable and useful without it.

### C. Direct `&mut` access to proxy bodies (like same-cluster entities)

Rejected. Proxies are read-only views of remote state. Allowing direct Rapier API calls on proxies would create the false impression that mutations are local; in reality, they must round-trip through Redis. The imperative-op interface (JSON routing) makes the RPC nature explicit.

### D. Synchronous RPC for ops instead of fire-and-forget

Rejected. Same-tick RPC (wait for the authority cluster to apply the op and return the result) would add one-cluster-latency to every cross-cluster op. Fire-and-forget is eventual consistency; if a game needs stricter semantics, it can build on top (bundle ops into a transaction, poll for confirmation via replication).

## Verification

```bash
# Docs compile and build checks pass
cargo build -p arcane-infra --features rapier-cluster
cargo test -p arcane-infra --features rapier-cluster

# Cross-cluster contact events + raycasts + ops routing
# (tested in sub-PR #132 — cross-cluster physics integration tests)
```

The 15 cross-cluster-specific tests in `rapier_cluster::tests::cross_cluster_*` cover:

- Proxy spawn/despawn (neighbor appears, disappears).
- Proxy position sync each tick (follows replicated position).
- Raycast against proxies (hit detection works).
- Collision events with proxies (contact surfaced both ways).
- Imperative ops on proxies (impulse routes to Redis, velocity change visible on authority next tick).
- Contact event flow-back (contact event published to authority cluster's physics_events topic).
- Entity_id dedup (local entities shadowed proxies with same ID, if any).
- Joint creation on proxies returns `None`.

## Consequences

### Positive

- Raycasts and intersection queries now work across cluster boundaries — necessary for client-side prediction (hitscan validation) and deterministic projectiles.
- Impulses and forces can cross cluster boundaries (with 1-tick latency) — enables physics interactions between entities in different clusters.
- Contact events are bidirectional — both clusters observe a cross-cluster collision.
- Defers the hard problem (authority transfer) to a later epic without blocking the common case (affinity prevents most cross-cluster interactions).

### Negative / accepted trade-offs

- **1-tick latency on ops** — a force applied to a proxy takes 1 cluster tick to propagate to the authority and return via replication delta.
- **Proxies are kinematic only** — no per-entity configurability yet. Affinity-clustered games can live with kinematic proxies; future configurability is tracked in [`#122`](https://github.com/brainy-bots/arcane/issues/122).
- **Fire-and-forget reliability** — ops can be lost if Redis crashes mid-publish. Affinity clustering (which minimizes cross-cluster interactions) makes this rare. Stricter semantics (transactional ops, acks, replay) are deferred to a follow-up ADR.
- **No cross-cluster joints yet** — prevents certain physics interactions (ragdolls spanning clusters, rope constraints). Affinity must co-locate joint participants. Deferred to authority-transfer epic.
- **Entity_id dedup rule** — if affinity is wrong and two clusters claim the same entity, local takes precedence. This is deterministic but silent; it's a symptom of an affinity bug, not a feature. Documented.

### Open follow-ups (tracked elsewhere)

- [`#119`](https://github.com/brainy-bots/arcane/issues/119) — Terrain epic; terrain colliders already support neighbor reads via `RapierMapProvider`.
- [`#122`](https://github.com/brainy-bots/arcane/issues/122) — Rapier gap inventory; lists proxy configurability and other capabilities.
- [`#135`](https://github.com/brainy-bots/arcane/issues/135) — Affinity clustering epic (planned); entity migration + authority transfer as a sub-epic.

## References

- [ADR-001](001-rapier-cluster-integration-shape.md) — Rapier cluster integration shape.
- [`docs/architecture/physics-backends-and-unreal.md`](../physics-backends-and-unreal.md) — Physics-tick contract and multi-backend path; cross-cluster section updated.
- [`docs/architecture/entity-model.md`](../entity-model.md) — Entity ownership, lifecycle, affinity clustering.
- [`issue #127`](https://github.com/brainy-bots/arcane/issues/127) — Cross-cluster physics epic (design conversation).
- [`crates/arcane-infra/src/rapier_cluster.rs`](https://github.com/brainy-bots/arcane/blob/main/crates/arcane-infra/src/rapier_cluster.rs) — Module docs, cross-cluster section.
