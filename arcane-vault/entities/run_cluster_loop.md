---
type: entity
tags: [arcane, arcane-infra, cluster, api, async, rust, library-architecture, tick-loop, entity-supplier]
---

# run_cluster_loop

## What It Is
`run_cluster_loop<F>` is the primary entry-point API in `arcane-infra` for driving a cluster server's tick loop. It accepts an optional per-tick entity supplier function, allowing callers to inject game-specific behavior (spawning entities, applying gravity, running AI) while the infrastructure handles clustering, replication, and WebSocket management.

## Origin & Evolution
The function emerged during the session documented in [[Project documentation overview]] (2026-03-03), as part of a deliberate architectural split between library concerns and demo-specific logic. Prior to this refactor, game-specific behavior such as gravity, jumping, and wandering agents was embedded directly in `arcane-infra`, making the library appear demo-first and limiting its credibility as a general-purpose multiplayer backend. Introducing `run_cluster_loop<F>` with a generic supplier parameter allowed `arcane-infra` to be a pure clustering and replication library while a new `arcane-demo` crate could own all game-specific tick behavior. This change also enabled two distinct binaries: `arcane-cluster` (pure infrastructure) and `arcane-cluster-demo` (demo behavior layered on top).

## Technical Details
- Generic over a supplier function `F` that is called once per tick; the return type provides entities or state updates to merge into the cluster's replicated world.
- The `F` parameter is optional — when absent, the loop runs pure infrastructure with no game-logic injection, making `arcane-cluster` binary viable.
- Lives in `arcane-infra`, which owns `ClusterManager`, `ClusterServer`, replication, and the WebSocket layer.
- Drives the async tick loop that coordinates spatial indexing (via `arcane-spatial`), rules-engine decisions (via `arcane-rules`), and outbound replication to connected clients.
- The generic design keeps `arcane-infra` free of any concrete game type imports while still being extensible at the binary level.

## Key Design Decisions
- **Generic entity supplier rather than trait object** — keeps zero-cost abstraction at the call site and avoids heap allocation per tick in hot-path server code.
- **Optional supplier (`Option<F>` or default no-op)** — allows the same function signature to power both the bare infrastructure binary and the demo binary without code duplication.
- **Separation from `arcane-demo` crate** — game-specific logic (gravity, wandering, jumping) lives in `arcane-demo`, not in the function itself, preserving `arcane-infra` as a library-grade component suitable for arbitrary game types.
- **Single entry point for the cluster binary** — centralising the loop in one function makes it easier to add cross-cutting concerns (metrics, backpressure, graceful shutdown) without scattering them across binaries.

## Relationships
- [[arcane-infra]] — crate that owns this function
- [[arcane-demo]] — the crate that provides the concrete supplier passed to this function in demo builds
- [[ClusterServer]] — the server instance whose tick loop `run_cluster_loop` drives
- [[ClusterManager]] — coordinates with the cluster server managed inside the loop
- [[arcane-cluster]] — binary that calls `run_cluster_loop` with no supplier (pure infrastructure)
- [[arcane-cluster-demo]] — binary that calls `run_cluster_loop` with the demo entity supplier
- [[SpatialIndex]] — queried each tick inside the loop for neighbor discovery
- [[RulesEngine]] — consulted each tick for clustering decisions

## Conversations That Shaped This
- [[Project documentation overview]] — the session where the `run_cluster_loop<F>` API was introduced and the library/demo split was executed