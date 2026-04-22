---
type: entity
tags: [architecture, clustering, scalability, infrastructure, node-types, tiered-systems]
---

# Heterogeneous Node Tiers

## What It Is
Heterogeneous Node Tiers describes the architectural pattern in Arcane where different node types perform fundamentally different roles within the cluster — specifically the split between **ClusterManager** (coordination, HTTP join, session routing) and **ClusterServer** (high-frequency simulation, WebSocket connections, physics/AI ticks). Rather than a homogeneous pool of identical servers, Arcane deliberately separates concerns across node types so each tier can be optimized, scaled, and replaced independently.

## Origin & Evolution
This design emerged from the core problem Arcane set out to solve: dedicated game-engine servers are real but single-process, while backend-as-a-service platforms scale but sacrifice physics fidelity. To break this tradeoff, Arcane needed a separation where simulation-heavy work lives on one tier and coordination/persistence lives on another. The architecture review session of 2026-03-02 crystallized the split: **ClusterServers** own high-frequency simulation state (movement, physics, AI), while a **ClusterManager** tier handles player join flows, session routing, and cluster-level bookkeeping. A third implicit tier — **SpacetimeDB** — serves as the authoritative persistent store for discrete game actions and durable state, further relieving ClusterServers of persistence responsibility.

## Technical Details
- **ClusterManager** (`arcane-infra`, binary `arcane-manager`): exposes an HTTP join endpoint; routes incoming players to appropriate ClusterServers; tracks cluster-level topology. Runs as a lightweight coordination process, not a simulation process.
- **ClusterServer** (`arcane-infra`, binary `arcane-cluster`): maintains WebSocket connections to clients; runs owned-entity simulation ticks (physics, AI, movement); writes owned entity state; communicates with Redis for cross-server replication. Does **not** execute game logic — that lives in SpacetimeDB reducers.
- **SpacetimeDB tier**: holds persistent game state and handles discrete game actions via reducers; ClusterServers write to it but are not the source of truth for durable state.
- **Redis**: sits between ClusterServer nodes as a replication bus, enabling owned-entity state from one node to fan out to neighbors without direct TCP RPC between cluster nodes.
- Crate responsibilities: `arcane-core` provides shared traits/types across tiers; `arcane-rules` (`RulesEngine`) drives clustering decisions that may affect node assignment; `arcane-pool` (`LocalPool`) manages the server pool abstraction.

## Key Design Decisions
- **Game logic in SpacetimeDB, not ClusterServers** — eliminates TCP RPC between cluster nodes for game actions; ClusterServers stay focused on simulation throughput
- **HTTP join on Manager, WebSocket on Cluster** — separates session setup latency path from hot simulation path; each can scale independently
- **Redis as replication bus between ClusterServers** — avoids point-to-point cluster mesh; allows owned-entity state to propagate without ClusterServer-to-ClusterServer coupling
- **No homogeneous pool** — each tier has a distinct binary (`arcane-manager`, `arcane-cluster`), making deployment topology explicit rather than implicit

## Relationships
- [[ClusterManager]]
- [[ClusterServer]]
- [[SpacetimeDB Integration]]
- [[Redis Replication Bus]]
- [[RulesEngine]]
- [[LocalPool]]
- [[arcane-infra]]
- [[arcane-core]]
- [[Replication Topology]]
- [[Entity Ownership]]

## Conversations That Shaped This
- [[Network library architecture review]]