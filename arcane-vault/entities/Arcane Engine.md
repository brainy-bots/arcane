---
type: entity
tags: [arcane, multiplayer-backend, rust, library, clustering, replication, infrastructure, core-concept]
---

# Arcane Engine

## What It Is
Arcane is a Rust multiplayer backend library providing cluster management, spatial partitioning, replication, and a reference server implementation. It is designed as a standalone, engine-agnostic infrastructure layer that replaces or supplements dedicated game-engine servers, allowing studios using Unreal, Unity, Godot, or custom engines to build on a shared backend. Its primary role in the project is to be the authoritative runtime for high-frequency simulation — movement, physics, AI ticks — while delegating persistent game state to SpacetimeDB.

## Origin & Evolution
Arcane emerged from the observation that existing multiplayer backend options force studios into a bad trade-off: dedicated game-engine servers offer real physics but are capped to single-process player counts, while backend-as-a-service platforms (SpacetimeDB, Nakama) scale but sacrifice simulation fidelity. The earliest concrete work was the PGP (Player Globe Partitioning) benchmark, which empirically tested whether clustering by social affinity (guilds, parties) outperforms raw spatial clustering for cross-cluster communication — a core architectural hypothesis. From there, the project evolved through several phases: initial toolchain setup (MSVC/Rust), a four-interface design (`IClusteringModel`, `IServerPool`, `IReplicationChannel`, `IWorldSimulator`), a major architecture review that resolved ten key design tensions, and finally a clean separation of library code from demo-specific logic with the creation of a dedicated `arcane-demo` crate.

## Technical Details
Arcane is organized as a Cargo workspace with five crates:

| Crate | Responsibility |
|---|---|
| `arcane-core` | Traits and shared types; no I/O |
| `arcane-spatial` | `SpatialIndex` — 2D grid for neighbor discovery |
| `arcane-rules` | `RulesEngine` — clustering decisions |
| `arcane-pool` | `LocalPool` — server pool implementation |
| `arcane-infra` | `ClusterManager`, `ClusterServer`, replication; binaries `arcane-cluster` and `arcane-manager` |

The system topology has clients connecting over WebSocket to `ClusterServer` nodes, which are coordinated by a `ClusterManager` (HTTP join). Redis is used for cross-cluster state propagation; SpacetimeDB holds authoritative persistent game state. The `run_cluster_loop<F>` API allows per-tick entity suppliers, enabling optional demo behavior without polluting the core library. A four-bucket data classification model — **Spine**, **Replicated**, **Ephemeral**, and **Persistent** — defines how entity data flows through the system: simulation concerns route to Arcane; persistence concerns route to SpacetimeDB.

## Key Design Decisions
- **Game logic lives in SpacetimeDB reducers, not ClusterServers** — ClusterServers own high-frequency simulation; SpacetimeDB owns discrete game actions and persistence. This eliminated the need for TCP RPC between clusters for game actions and simplified failover.
- **Four-bucket data model over per-property replication flags** — Reduces wire metadata, makes replication rules explicit at the type level, and is easier for developers to reason about than Unreal-style flag annotations.
- **Engine-agnostic, standalone library** — Unreal (and any other engine) is a consumer client, not the host. Early plugin-first instincts were corrected in favor of a standalone Rust library with a thin client plugin layer.
- **Social-affinity clustering (PGP) over raw spatial clustering** — Validated by benchmark that grouping players by guild/party relationships reduces cross-cluster communication overhead compared to pure proximity bucketing.
- **Hysteresis thresholds for clustering stability** — Prevents oscillation (merge-then-split loops) in naive threshold-based clustering systems; server load is used as a scaling signal to spawn new servers rather than collapse clusters.
- **`arcane-demo` crate for demo isolation** — Separates game-specific demo behavior (gravity, jumping, wandering agents) from `arcane-infra`, making Arcane credible as a general-purpose library rather than a demo-first project.
- **AGPL-3.0 with commercial license option** — Network-use copyleft by default; commercial license available for proprietary deployments.

## Relationships
- [[ClusterManager]]
- [[ClusterServer]]
- [[SpatialIndex]]
- [[RulesEngine]]
- [[LocalPool]]
- [[SpacetimeDB Integration]]
- [[Redis Replication Layer]]
- [[arcane-client-unreal]]
- [[Four-Bucket Data Model]]
- [[PGP Clustering]]
- [[run_cluster_loop API]]

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]]
- [[Unreal Engine setup for networking library]]
- [[Untitled Chat (2026-02-24)]]
- [[Network library architecture review]]
- [[Project documentation overview]]
- [[Untitled Chat (2026-03-03)]]