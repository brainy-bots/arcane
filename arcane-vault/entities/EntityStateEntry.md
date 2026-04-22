---
type: entity
tags: [state-replication, entity-state, data-model, arcane-infra, arcane-core, cluster-server, tick-loop, rust]
---

# EntityStateEntry

## What It Is
`EntityStateEntry` is the per-entity snapshot type that captures the state of a single game entity at a point in time within Arcane's replication pipeline. It serves as the atomic unit of state inside `EntityStateDelta`, which is constructed each tick by `ClusterServer` and broadcast to all connected clients. Every property that must be communicated to clients — position, velocity, health, or any other replicated field — is encoded in an `EntityStateEntry`.

## Origin & Evolution
The type emerged from the need to support a **broadcast-first, serialize-once** replication model without re-serializing per client. Early design pressure came from asking: what is the minimal, self-contained representation of entity state that can be assembled into a delta, merged with neighbor cluster data, serialized once to JSON, and dropped into a broadcast channel for all WebSocket subscribers? `EntityStateEntry` is the answer — a flat, owned record that can be moved through mpsc channels and cloned cheaply enough to populate a `broadcast::Sender`. Work documented in the `STATE_UPDATE message handling` session confirmed that no per-client filtering or re-serialization happens in the hot path, which locked in the design requirement that an `EntityStateEntry` must carry everything any client might need, since there is no later stage to enrich it.

## Technical Details
- Defined in **arcane-core** (no I/O crate) so it is available to all downstream crates without introducing circular dependencies.
- Consumed by **arcane-infra** inside `ClusterServer`'s tick loop: the server iterates its local entity map, constructs one `EntityStateEntry` per entity, and assembles the collection into an `EntityStateDelta`.
- The delta (containing a `Vec<EntityStateEntry>` for live entities plus a pending-removals list) is pushed over an **mpsc channel** to the WebSocket server task, serialized to JSON exactly once, then placed into a **tokio broadcast channel** from which every client task reads the same bytes.
- Neighbor cluster data is merged into the delta before serialization, meaning `EntityStateEntry` values from remote clusters travel the same path as local ones — the struct must be representable for both local and neighbor-sourced entities.
- Fields are expected to include at minimum: entity ID, position, and any gameplay-relevant scalar fields (velocity, health, etc.), though the exact schema evolves with the physics and data-model work tracked in benchmark sessions.

## Key Design Decisions
- **Flat, owned struct** — enables cheap movement through channel boundaries without lifetime complications in async tasks.
- **Defined in arcane-core** — keeps the canonical state shape decoupled from I/O and infrastructure, making it testable in isolation and reusable by rules and spatial crates.
- **No per-client variant** — the broadcast model means a single `EntityStateEntry` shape must satisfy all subscribers; per-client culling or projection is a future concern noted as a future insertion point in the replication channel.
- **Merged with neighbor data before serialization** — simplifies the WebSocket layer by presenting a unified world snapshot rather than requiring clients to stitch together per-cluster streams.

## Relationships
- [[EntityStateDelta]]
- [[ClusterServer]]
- [[ReplicationChannel]]
- [[SpatialIndex]]
- [[arcane-core]]
- [[arcane-infra]]
- [[BroadcastChannel]]

## Conversations That Shaped This
- [[STATE_UPDATE message handling in ClusterServer]]
- [[Benchmark improvement suggestions]]