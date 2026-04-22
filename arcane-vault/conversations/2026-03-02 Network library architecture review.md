---
type: conversation
date: 2026-03-02
source: cursor
tags: [architecture, rust, replication, redis, unreal-engine, tdd, spacetimedb, clustering, websocket, infra]
---

# Network library architecture review

**Date:** 2026-03-02
**Source:** cursor (4029 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/cursor-chat-export-pgp-demo/2026-03-02-16-Network_library_architecture_r.md`

## Summary

This session was a comprehensive architecture review, documentation overhaul, and implementation sprint for the Arcane multiplayer backend library. The primary goal was to resolve fundamental design tensions in the distributed clustering system — covering state ownership, replication topology, game logic placement, entity lifecycle, clustering cadence, and failover — producing a unified architecture that is coherent enough to build production software against.

A key architectural decision was made early: game logic lives in **SpacetimeDB reducers**, not in ClusterServers. ClusterServers handle high-frequency simulation (movement, physics, AI ticks) and write owned entity state; SpacetimeDB is the single authoritative source for persistent game state and discrete game actions. This decision eliminated the need for TCP RPC between clusters for game actions and simplified the overall design significantly. Ten major tension points were resolved in the architecture docs, and an observability-first posture was added as a required production consideration.

On the implementation side, the session followed a strict TDD approach across the Rust workspace. Five crates were scaffolded and implemented: `arcane-core` (interfaces and shared types), `arcane-spatial` (2D spatial index, 9 tests passing), `arcane-rules` (static clustering model stub), `arcane-pool` (server pool allocator), and `arcane-infra` (ClusterManager, ClusterServer, ReplicationChannelManager). Redis-backed replication via pub/sub was implemented and integration-tested, cluster server tick loops were wired to broadcast `EntityStateDelta` to neighbors, and runnable binaries (`arcane-cluster`, `arcane-manager`) were created with environment-variable configuration.

The session concluded with a full Unreal Engine in-game demo: an `ArcaneClient` plugin with an `UArcaneAdapterSubsystem` handling HTTP join, WebSocket connection, and entity caching; an `ArcaneDemo` game project with an `AArcaneEntityDisplay` actor rendering debug spheres per entity; and a companion PowerShell script (`run_demo.ps1`) to start the full backend stack locally. A visualization demo (outside UE) was also produced — 500-frame animated HTML showing 16 player groups and 18 solos with interaction-driven cluster assignments colored by cluster ID.

## What Was Built

- `arcane-core` crate: interfaces (`IClusteringModel`, `IServerPool`, `IReplicationChannel`, `IWorldSimulator`) and shared types (`Vec2/3`, `ClusterGeometry`, `WorldStateView`)
- `arcane-spatial` crate: in-memory 2D spatial index with 9 passing unit tests
- `arcane-rules` crate: static rule-based clustering model stub, ready for real rules
- `arcane-pool` crate: server pool allocator with handle provisioning and `PoolExhausted` error handling
- `arcane-infra` crate: `ClusterManager`, `ClusterServer`, `ReplicationChannelManager`, `RpcHandler` (marked optional/non-game)
- `RedisReplicationChannel`: Redis pub/sub on `arcane:replication:{cluster_id}` with round-trip integration test
- `ReplicationChannelManager`: neighbor tracking, single broadcast channel per cluster, `send_to_neighbors(delta)`
- `ClusterManager` topology methods: `get_neighbors_for_cluster()`, `set_observation_radius()`
- `arcane-cluster` binary: 20 Hz tick loop, env-var config (`CLUSTER_ID`, `REDIS_URL`, `NEIGHBOR_IDS`), `DEMO_ENTITIES=N` seeding
- `arcane-manager` binary: HTTP `/join` endpoint returning `cluster_id`, `server_host`, `server_port`
- Optional `cluster-ws` feature: WebSocket broadcast of `STATE_UPDATE` (EntityStateDelta as JSON) per tick
- In-memory entity store in `ClusterServer`: `add_entity`, `remove_entity`, real `updated`/`removed` delta lists
- `scripts/run_demo.ps1`: starts manager + cluster together for local demo
- `ArcaneClient` Unreal plugin: `UArcaneAdapterSubsystem` with HTTP join, WebSocket connection, entity cache, `GetEntitySnapshot()`
- `ArcaneDemo` UE game project: `AArcaneEntityDisplay` actor drawing debug spheres per entity (100x scale)
- 500-frame animated `viz.html` + `state.json` showing interaction-driven clustering with 34 entities
- 16 markdown architecture/interface/component docs covering IF-01–04, IN-01–07, CA-01–02, end-to-end flows, schema, best practices
- "Game logic placement" section added to architecture index under §6 Production path and known limitations
- Observability requirements section added to architecture docs (correlation IDs, structured logging, metrics, failure injection)
- Docker Compose setup: Redis 7 on port 6379 for local dev

## Key Decisions

- **Game logic in SpacetimeDB reducers, not ClusterServers**: ClusterServers run simulation only; reducers are the authoritative path for combat, inventory, spells, and all discrete game actions. Eliminates TCP RPC for game actions; simplifies library scope; enables time-travel replay and audit.
- **RPCHandler marked optional/non-game**: TCP RPC between clusters is not needed for game logic under this architecture; retained as an optional future performance dial.
- **Redis pub/sub for replication**: State deltas broadcast via `arcane:replication:{cluster_id}`; no direct server-to-server references. Clean decoupling; pub/sub scales horizontally.
- **Ownership on SpacetimeDB**: Only the owning cluster writes an entity's state; ownership records live in SpacetimeDB, not in-process. Single source of truth prevents write conflicts.
- **Interaction-driven clustering**: Clusters form based on likelihood of interaction (guild, party, enemy relationships) + relative position, not absolute position alone. Load-based splits (max entities per cluster) prevent degenerate large clusters.
- **Periodic clustering cadence, tunable**: Clustering runs on a configurable timer; ML inference can be remote, not in the hot path.
- **ClusterManager failover via read-evaluate-write**: No in-flight state means failover is safe; any new manager can pick up from SpacetimeDB.
- **FastForward window capped per call**: Prevents unbounded catch-up cost; periodic low-rate updates fill gaps.
- **Removed entity detection via seq + gap detection → full sync**: Missing sequence numbers trigger a full sync from SpacetimeDB rather than relying on server-side commit coordination.
- **Observability as a first-class requirement**: Correlation IDs, structured logging, metrics, and failure injection must be present from day one; not deferred to a production playbook.
- **TDD throughout**: Scaffolding → failing tests → implementation → refactor for all crates; 27 tests passing across the workspace at session close.
- **UE 5.7.3 target**: Demo project rebuilt targeting Unreal Engine 5.7; `.NET Framework SDK` required for SwarmInterface module.

## Problems Solved

- Resolved all 10 major architecture tension points: merge/split coordination, server reuse without direct references, removed entity handling, write ownership conflicts, entity instantiation for unobserved entities, FastForward unboundedness, neighbor topology consistency, RPC vs. state semantics, clustering cadence scalability, and ClusterManager failover safety.
- Disambiguated game logic placement — prior docs were ambiguous about whether ClusterServers ran game logic; now explicit.
- Fixed Redis integration test delivery quirk: same-process pub/sub doesn't deliver to self; documented workaround using a second publish on a fresh connection.
- Fixed UE 5.7 HTTP API breaking changes: updated includes to `IHttpRequest.h`, `IHttpResponse.h`; made mutex `mutable` for const method locking.
- Resolved Unreal plugin load failures (referenced in Task 15).
- Removed Unreal-specific setup from core repo so the Rust workspace builds from scratch without UE toolchain.

## Entities

- [[Arcane Engine]]
- [[PGP Architecture]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[SpaceTimeDB]]
- [[Unreal Engine Client]]
- [[Redis]]
- [[Spatial Grid]]
- [[arcane-scaling-benchmarks]]
- [[arcane-demos]]
- [[arcane-client-unreal]]
- [[Heterogeneous Node Tiers]]
- [[Affinity Clustering]]

NEW:
- NEW: [[RedisReplicationChannel]]
- NEW: [[ReplicationChannelManager]]
- NEW: [[arcane-core]]
- NEW: [[arcane-spatial]]
- NEW: [[arcane-rules]]
- NEW: [[arcane-pool]]
- NEW: [[arcane-infra]]
- NEW: [[UArcaneAdapterSubsystem]]
- NEW: [[ArcaneDemo]]
- NEW: [[EntityStateDelta]]
- NEW: [[IWorldSimulator]]
- NEW: [[IClusteringModel]]
- NEW: [[IServerPool]]
- NEW: [[IReplicationChannel]]

## Related Conversations

_to be linked_