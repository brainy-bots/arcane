---
type: entity
tags: [physics, scale, architecture, simulation, multiplayer, dedicated-servers, cluster]
---

# Physics at Scale

## What It Is
Physics at Scale refers to Arcane's core architectural challenge and value proposition: enabling real, combat-grade physics simulation across player counts that exceed what any single dedicated game-engine server process can handle. It is the fundamental tension the entire Arcane project exists to resolve — preserving simulation fidelity while distributing load across a cluster of servers.

## Origin & Evolution
The problem crystallized from the two-bad-choices problem in the multiplayer backend market: dedicated game-engine servers (Unreal, Unity) offer real physics but are capped to a single process and a fixed player count, while WASM-based backend-as-a-service platforms (SpacetimeDB, Nakama) offer scale but sacrifice simulation fidelity. Arcane was designed from the ground up to break that tradeoff — distributing simulation across multiple `ClusterServer` nodes while maintaining enough spatial and state coherence to support real physics and combat interactions. The `arcane-spatial` crate (SpatialIndex, 2D grid for neighbor discovery) and `arcane-rules` crate (RulesEngine for clustering decisions) exist directly in service of this goal.

## Technical Details
The architecture separates concerns across crates specifically to support distributed simulation:

- **arcane-spatial**: A 2D spatial grid (`SpatialIndex`) enabling efficient neighbor discovery — the foundation for deciding which entities are physically relevant to each other across server boundaries.
- **arcane-rules**: A `RulesEngine` that makes clustering decisions, determining how entities, players, and simulation zones are assigned to `ClusterServer` instances.
- **arcane-infra**: `ClusterManager` and `ClusterServer` handle replication and coordination; multiple `ClusterServer` nodes collectively simulate what no single server could.
- **arcane-pool**: `LocalPool` manages the server pool, supporting horizontal scaling of simulation capacity.
- Redis is used for inter-node state sharing and replication, allowing simulation state to cross server boundaries without full round-trips through the client.

The `ClusterManager` acts as the authoritative coordinator, with `ClusterServer` nodes handling WebSocket connections and local simulation, replicating relevant state via Redis.

## Key Design Decisions
- **Cluster-first architecture over single-process** — breaks the player-count ceiling imposed by dedicated game-engine servers; rationale is the core WHY_ARCANE problem statement
- **Spatial indexing as a first-class crate (arcane-spatial)** — neighbor discovery must be fast and decoupled from business logic to make zone/shard boundaries computationally tractable
- **Rules engine as a separate crate (arcane-rules)** — clustering decisions (who goes where) are policy, not mechanism; keeping them isolated allows tuning without touching simulation or networking code
- **Engine-agnostic client interface** — physics authority lives in Arcane, not in any one engine, so Unity, Unreal, Godot, and custom clients can all connect without re-implementing simulation
- **Redis for replication** — chosen for low-latency pub/sub and shared state across cluster nodes, avoiding direct peer-to-peer coupling between ClusterServers

## Relationships
- [[ClusterManager]]
- [[ClusterServer]]
- [[SpatialIndex]]
- [[RulesEngine]]
- [[LocalPool]]
- [[Replication]]
- [[arcane-spatial]]
- [[arcane-rules]]
- [[arcane-infra]]
- [[arcane-pool]]
- [[WHY_ARCANE — Positioning & Problem Statement]]

## Conversations That Shaped This
- No direct conversation excerpts found — core framing inferred from `WHY_ARCANE.md` and `README.md` repo context