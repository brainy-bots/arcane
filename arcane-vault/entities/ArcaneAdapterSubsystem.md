---
type: entity
tags: [architecture, rust, infra, library-design, api, clustering, replication, extensibility]
---

# ArcaneAdapterSubsystem

## What It Is
The ArcaneAdapterSubsystem is the architectural layer in `arcane-infra` that separates pure infrastructure concerns (clustering, replication, WebSocket transport, Redis pub/sub) from game-specific or demo-specific logic. It defines the boundary through which external behavior — entity suppliers, tick callbacks, game logic hooks — is injected into the cluster runtime without polluting the library core.

## Origin & Evolution
The subsystem emerged from a structural tension identified in the 2026-03-03 session: `arcane-infra` had begun accumulating demo-specific logic (gravity, jumping, wandering agents) that made it appear to be a demo-first project rather than a general-purpose library. The resolution was to introduce a clean injection API — `run_cluster_loop<F>` — that accepts an optional per-tick entity supplier as a generic parameter. This allowed a dedicated `arcane-demo` crate to house all game-specific behavior while `arcane-infra` remained a pure clustering and replication substrate. Two binary targets reinforced the split: `arcane-cluster` (pure infrastructure) and `arcane-cluster-demo` (infrastructure + demo behavior wired in via the adapter boundary).

## Technical Details
The core of the subsystem is the `run_cluster_loop<F>` generic function in `arcane-infra`, which parameterizes the cluster tick loop over a caller-supplied closure or type that provides entities on each tick. The function signature allows `None` to be passed when no game logic is needed, making the binary useful as a bare infrastructure node. The adapter boundary is enforced at the crate level: `arcane-infra` has no dependency on `arcane-demo`, and all game-specific state (agent positions, demo wandering logic, jump physics) lives exclusively in `arcane-demo`. The split is backed by two separate Cargo binaries in the workspace, so deployment targets are distinct artifacts.

Key interfaces:
- **`run_cluster_loop<F>`** — generic entry point accepting an optional per-tick supplier `F: Fn() -> Vec<Entity>`
- **`arcane-infra` crate** — pure library: ClusterManager, ClusterServer, replication, WebSocket, Redis
- **`arcane-demo` crate** — consumer of the adapter API; injects gravity, wandering, demo agents
- **`arcane-cluster` binary** — infra only, no game logic
- **`arcane-cluster-demo` binary** — infra + demo behavior via adapter

## Key Design Decisions
- **Generic tick supplier over trait object** — using `run_cluster_loop<F>` with a generic bound keeps zero-cost abstraction and avoids heap allocation on the hot tick path
- **`None` as a valid supplier** — allows the same API to serve both pure-infra deployments and game-logic deployments without forking the function
- **`arcane-demo` as a separate crate, not a feature flag** — feature flags would have left demo code inside `arcane-infra`'s source tree, undermining the credibility of the library boundary
- **Two distinct binaries** — `arcane-cluster` and `arcane-cluster-demo` make the separation visible at the deployment level, not just the source level
- **No upward dependency** — `arcane-infra` must not import `arcane-demo`; the dependency arrow points only downward, preserving `arcane-infra` as a reusable library crate

## Relationships
- [[ArcaneInfra]]
- [[ArcaneDemo]]
- [[ClusterServer]]
- [[ClusterManager]]
- [[RunClusterLoop]]
- [[ArcaneCore]]
- [[ReplicationSubsystem]]

## Conversations That Shaped This
- [[Project documentation overview]] — primary session where the library/demo split was designed and `run_cluster_loop<F>` was introduced
- [[Network library architecture review]] — established the broader principle that game logic (SpacetimeDB reducers) and infrastructure (ClusterServers) must be kept separate, which motivated the adapter boundary
- [[Standalone binary for Unreal Engine testing]] — validated the two-binary approach in a production benchmarking context where `arcane-cluster` ran as a pure infra node