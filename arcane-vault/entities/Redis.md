---
type: entity
tags: [redis, infrastructure, replication, pub-sub, state-sync, clustering, arcane-infra]
---

# Redis

## What It Is
Redis is the shared message bus and state synchronization layer for the Arcane distributed cluster system. It sits between ClusterServers, enabling cross-cluster entity visibility and state broadcast without requiring direct TCP connections between individual cluster nodes. In the Arcane architecture it is one of two external dependencies the `arcane-infra` crate integrates with — the other being SpacetimeDB.

## Origin & Evolution
Redis entered the design as part of the core clustering architecture established during the **Network library architecture review** session. The problem it solves is fundamental to multi-cluster simulation: when entity state (position, health, animation) must be visible across server boundaries at high frequency, point-to-point RPC between clusters does not scale. Redis pub/sub provides a fan-out primitive that decouples writers (ClusterServers publishing entity ticks) from readers (other ClusterServers and the ClusterManager subscribing to relevant channels). An earlier benchmark iteration (the PGP demo) used TCP RPC for cross-cluster attacks; the architectural review explicitly eliminated that pattern for game actions, routing discrete game events to SpacetimeDB and keeping Redis as the high-frequency simulation bus.

## Technical Details
Redis is consumed by the `arcane-infra` crate and is required at runtime for the `arcane-cluster` binary (`cargo run -p arcane-infra --bin arcane-cluster --features cluster-ws`). The replication topology has ClusterServers publishing owned-entity state snapshots to Redis channels at the simulation tick rate; other cluster nodes subscribe to channels covering their spatial neighborhood to receive remote entity state. The ClusterManager also reads from Redis to maintain a global view of entity distribution across the pool. The four-bucket data model classifies this traffic as **Replicated** data — high-frequency, authoritative for the owning cluster, ephemeral in the sense that it is not durably stored. Redis is explicitly *not* used for **Persistent** data (discrete game actions, inventory, economy), which routes to SpacetimeDB.

## Key Design Decisions
- **Pub/sub over point-to-point RPC** — earlier benchmark work used TCP RPC for cross-cluster communication; the architecture review replaced this with Redis fan-out to avoid O(n²) connection topology as cluster count grows
- **Redis scoped to Replicated bucket only** — the four-bucket model (Spine, Replicated, Ephemeral, Persistent) keeps Redis off the critical path for durable state; SpacetimeDB owns persistence, Redis owns simulation broadcast
- **ClusterServer as publisher, not ClusterManager** — each ClusterServer publishes its own owned entities directly, keeping the ClusterManager in a supervisory role rather than a data relay, which reduces central bottleneck risk
- **Required feature flag** — Redis support is gated behind `cluster-ws` feature in `arcane-infra`, keeping the core library and manager binary free of the Redis dependency when not needed

## Relationships
- [[ClusterServer]]
- [[ClusterManager]]
- [[arcane-infra]]
- [[SpacetimeDB]]
- [[Replication]]
- [[Four-Bucket Data Model]]
- [[SpatialIndex]]

## Conversations That Shaped This
- [[Network library architecture review]]
- [[Specification implementation for concept demonstration]]
- [[Untitled Chat]]
- [[Standalone binary for Unreal Engine testing]]
- [[Benchmark improvement suggestions]]