---
type: entity
tags: [state-model, architecture, replication, game-state, design, arcane-core]
---

# Four-Bucket State Model

## What It Is
The Four-Bucket State Model is Arcane's canonical partitioning of all game-world data into four distinct categories based on ownership, visibility, and replication semantics. It provides a principled framework for deciding where state lives, who can write it, and how it propagates across the cluster — replacing ad-hoc per-feature decisions with a single consistent model.

## Origin & Evolution
The model emerged from the broader architectural work on Arcane's data model and physics architecture, surfaced during the benchmark improvement and scripting infrastructure sessions. The core problem it solves is the classic multiplayer state-management question: not every piece of game data should be replicated the same way or owned by the same authority. Without explicit partitioning, replication logic becomes entangled with game logic and physics authority becomes ambiguous. The four-bucket taxonomy was established to give the team a shared vocabulary and a set of enforceable contracts that the replication layer (arcane-infra) and rules engine (arcane-rules) can reason about consistently.

## Technical Details
The model partitions all state into four buckets:

1. **Authoritative Server State** — owned and written exclusively by the ClusterServer; the ground truth for simulation. Physics simulation results, entity positions post-reconciliation, and combat outcomes live here. No client write access.

2. **Client-Predicted State** — state that clients speculatively advance locally (movement, input-driven actions) before server confirmation. The server reconciles this against authoritative state and issues corrections.

3. **Replicated Shared State** — state broadcast to multiple clients based on spatial interest (driven by [[SpatialIndex]]). Neighbor discovery determines which entities receive which slices. Read-only on receiving clients.

4. **Ephemeral / Session State** — transient state that is not durably persisted; lives for the lifetime of a connection or match. Examples include transient effect flags and in-flight input queues. Not stored in Redis or SpacetimeDB.

Each bucket has defined replication semantics wired into [[ClusterManager]] and [[ClusterServer]], and the [[RulesEngine]] uses bucket membership to drive clustering decisions (e.g., which entities must be co-located on the same server for authoritative physics).

## Key Design Decisions
- **Explicit ownership per bucket** — eliminates ambiguous write authority; the replication layer rejects writes that violate bucket ownership contracts
- **Spatial interest scoping for Replicated Shared State** — rather than broadcasting all shared state to all clients, [[SpatialIndex]] gates what is replicated to whom, keeping bandwidth proportional to local density
- **Ephemeral bucket kept out of persistence layer** — session state is never written to Redis or SpacetimeDB, avoiding persistence overhead for data that is inherently short-lived
- **Client-Predicted bucket is server-reconciled, not server-ignored** — prediction without reconciliation produces divergence; the model mandates a correction path rather than trusting client state
- **Bucket taxonomy informs clustering** — the [[RulesEngine]] uses bucket semantics to decide when entities must share a server (authoritative co-simulation) vs. when they can be split (independent replicated state)

## Relationships
- [[ClusterManager]]
- [[ClusterServer]]
- [[RulesEngine]]
- [[SpatialIndex]]
- [[arcane-core]]
- [[arcane-infra]]
- [[arcane-rules]]
- [[Replication]]
- [[Physics Authority]]

## Conversations That Shaped This
- [[Benchmark improvement suggestions]]