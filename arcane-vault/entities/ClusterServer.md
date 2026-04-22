---
type: entity
tags: [arcane, infra, websocket, clustering, replication, rust, server, cluster-management]
---

# ClusterServer

## What It Is
ClusterServer is the runtime server node in Arcane's distributed clustering architecture, implemented in the `arcane-infra` crate. It handles high-frequency simulation — movement, physics, AI ticks — for the subset of players assigned to it, managing WebSocket connections from clients and writing owned entity state out to the replication layer. It is the "leaf" in the system: ClusterManager routes players to it; ClusterServer does the per-tick work.

## Origin & Evolution
ClusterServer emerged from the early PGP (Player Globe Partitioning) benchmark work (February 2026), where individual cluster nodes ran WebSocket simulation with TCP RPC for cross-cluster communication. At that stage, game logic was mixed into the server nodes, and cross-cluster attacks were handled via TCP RPC calls between cluster processes — an approach that proved messy and difficult to reason about.

The March 2026 architecture review resolved this: **game logic moved to SpacetimeDB reducers**, and ClusterServer was narrowed to pure simulation concerns (movement, physics, AI ticks). This eliminated inter-cluster TCP RPC for game actions and simplified the design substantially. A further refactor split demo-specific behavior (gravity, jumping, wandering agents) into a dedicated `arcane-demo` crate, leaving `arcane-infra`'s ClusterServer as a clean, general-purpose infrastructure component. The `run_cluster_loop<F>` API was introduced at this point to allow optional per-tick entity suppliers without coupling the binary to demo logic. Two binaries now exist: `arcane-cluster` (pure infrastructure) and `arcane-cluster-demo` (with demo behavior).

## Technical Details
- Lives in **`arcane-infra`**, built and run via `cargo run -p arcane-infra --bin arcane-cluster --features cluster-ws`
- Accepts **WebSocket connections** from clients (Unreal Engine plugin or other consumers)
- Runs a **per-tick simulation loop** over owned entities; the tick supplier is injectable via `run_cluster_loop<F>`
- Writes entity state to **Redis** for replication and cross-cluster state sharing; does not own persistent game state
- Persistent and discrete game actions are delegated to **SpacetimeDB reducers** — ClusterServer does not handle these directly
- Receives player assignments from **ClusterManager** (the HTTP coordination layer); does not self-assign players
- Observability (Prometheus metrics, tick rate, backpressure) is a required production concern — see `docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md` for channel backpressure behavior

## Key Design Decisions
- **Game logic lives in SpacetimeDB, not ClusterServer** — ClusterServer is a simulation engine, not an authority on game rules; this eliminated inter-cluster TCP RPC for game actions
- **Simulation only, no persistence** — ClusterServer writes to Redis for replication but SpacetimeDB is the single authoritative source for persistent game state
- **Injectable tick supplier (`run_cluster_loop<F>`)** — keeps `arcane-infra` general-purpose; demo-specific behavior is supplied by `arcane-demo`, not baked in
- **Two binaries** — `arcane-cluster` (library-clean) and `arcane-cluster-demo` (with demo agents) make Arcane credible as a general-purpose backend, not a demo-first project
- **WebSocket transport** — chosen for client connectivity (Unreal plugin, browser tools, etc.); backpressure behavior is explicitly documented and validated

## Relationships
- [[ClusterManager]] — assigns players to ClusterServers; the HTTP coordination layer above
- [[arcane-infra]] — the crate that contains ClusterServer
- [[arcane-demo]] — supplies demo-specific tick behavior to ClusterServer via `run_cluster_loop<F>`
- [[arcane-core]] — traits and shared types consumed by ClusterServer
- [[RulesEngine]] — clustering decisions (via `arcane-rules`) that determine which ClusterServer a player belongs to
- [[LocalPool]] — server pool abstraction (via `arcane-pool`) managing available ClusterServer instances
- [[SpatialIndex]] — neighbor discovery used in routing and clustering decisions
- [[Redis]] — replication and cross-cluster state sharing target
- [[SpacetimeDB]] — authoritative persistent game state; ClusterServer delegates game actions here

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]] — original PGP benchmark; first implementation of cluster server nodes with WebSocket simulation and TCP RPC
- [[Untitled Chat (2026-02-20)]] — surfaced architectural failures (metrics at zero, UI controls broken, hardcoded limits ignored) that revealed the initial design was not viable
- [[Network library architecture review]] — the pivotal session; resolved ten major design tensions including game logic placement, state ownership, and failover; locked in the SpacetimeDB-for-persistence decision
- [[Project documentation overview]] — refactored ClusterServer into a clean library component; introduced `run_cluster_loop<F>` and the `arcane-demo` split; established the two-binary structure