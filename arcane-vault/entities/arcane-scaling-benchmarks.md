---
type: entity
tags: [benchmarking, scaling, pgp, clustering, load-testing, prometheus, grafana, websocket, redis, spatial-grid, rust, hypothesis-testing]
---

# arcane-scaling-benchmarks

## What It Is
`arcane-scaling-benchmarks` is a standalone benchmark workspace used to empirically validate the core architectural hypotheses of the Arcane platform — specifically whether Player Globe Partitioning (PGP), which clusters players by social affinity (guilds, parties, enemy state), produces better cross-cluster communication characteristics than naive spatial clustering. It serves as both a proof-of-concept demonstration environment and a development staging ground where core Arcane architectural patterns were first prototyped and stress-tested before being formalized into the main library.

## Origin & Evolution
The benchmarks project originated from a PDF specification describing the PGP hypothesis: that social-affinity clustering would reduce cross-cluster traffic compared to pure spatial partitioning. Early sessions built the full stack from scratch — a Spatial Grid Server for baseline comparison, a PGP Cluster Manager with HTTP coordination, individual Cluster Servers with WebSocket simulation and TCP RPC for cross-cluster attacks, a multi-mode Load Generator, and a Prometheus + Grafana monitoring stack.

The project immediately surfaced real architectural failures. Despite a hardcoded 20-player-per-server limit, limits were not being enforced; the cluster server reported a consistent 1 kHz tick rate while the grid server flatlined at 0 Hz; most metrics stayed at zero despite active load; and UI controls for dynamic parameter adjustment reset to zero without effect. These failures were not incidental bugs — they revealed that the benchmark design itself was invalid for testing the hypothesis it was built to validate.

This crisis forced a productive pivot. The benchmarks workspace became the site where foundational design decisions for Arcane proper were first worked through: the four-interface architecture (`IClusteringModel`, `IServerPool`, `IReplicationChannel`, `IWorldSimulator`), the behavioral-metrics approach to clustering (interaction likelihood over raw proximity), hysteresis thresholds to prevent cluster oscillation, and server-load as a scaling signal. Later sessions used the repo as a development environment for the `arcane-client-unreal` toolchain setup and the broader architecture review that produced the production Arcane design.

## Technical Details
The benchmark suite comprised several cooperating services:

- **Spatial Grid Server** — baseline comparison implementation using 2D grid partitioning; feeds into [[arcane-spatial]] design
- **PGP Cluster Manager** — HTTP-coordinated manager with spatial indexing; prototype for [[arcane-infra]] `ClusterManager`
- **Cluster Servers** — WebSocket-based player simulation; early TCP RPC for cross-cluster communication (later eliminated in Arcane proper in favor of SpacetimeDB reducers for game actions)
- **Load Generator** — multi-mode; supports scripted and randomized player interaction patterns
- **Observability stack** — Prometheus metrics collection + Grafana dashboards; Docker-networked for service discovery

Key metric bugs discovered during development: tick rate emitting at ~1 kHz instead of the intended 20 Hz (incorrect timer logic), and cross-cluster RPC failures being silently misreported. Dynamic parameter controls (active player count, interactions-per-player, cluster configuration) were implemented via a web trigger panel but proved unreliable — controls reset without effect, blocking parameter-space exploration.

The workspace also housed the cursor chat exports that document the entire design evolution of Arcane, making it a de facto project journal as well as a benchmark suite.

## Key Design Decisions
- **Social affinity over spatial proximity for clustering** — the PGP hypothesis; players who interact (guild, party, combat) should share a cluster to minimize cross-cluster RPC, regardless of world position
- **Hysteresis thresholds on cluster merge/split** — prevents oscillation where clusters merge and immediately re-split under naive threshold systems; adopted after observing instability in the visualization demo
- **Server load as a scaling signal** — when player density converges, new servers spawn rather than collapsing into an overloaded cluster
- **Behavioral interaction-likelihood metrics** — clustering decisions grounded in who players are likely to interact with, not raw position; directly influenced [[arcane-rules]] `RulesEngine` design
- **TCP RPC for cross-cluster abandoned** — early benchmark used TCP RPC between cluster servers for game actions; architecture review concluded game logic belongs in SpacetimeDB reducers, eliminating this channel entirely
- **Observability-first posture** — Prometheus + Grafana integrated from the start; metric bugs (wrong tick rate, silent RPC failures) validated that observability must be a first-class requirement, not an afterthought

## Relationships
- [[arcane-core]] — four-interface pattern (`IClusteringModel`, `IServerPool`, `IReplicationChannel`, `IWorldSimulator`) first sketched in benchmark sessions
- [[arcane-spatial]] — SpatialIndex design informed by Spatial Grid Server baseline
- [[arcane-rules]] — RulesEngine clustering logic descended from PGP hypothesis validation work
- [[arcane-infra]] — ClusterManager and ClusterServer architecture prototyped here before formalization
- [[arcane-client-unreal]] — toolchain setup (MSVC, WSL vs Windows decision) occurred within this workspace
- [[pgp-clustering]] — the core hypothesis the benchmarks were built to validate
- [[spacetimedb-integration]] — decision to route game actions through SpacetimeDB reducers rather than TCP RPC emerged from benchmark failure analysis

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]] (2026-02-20) — initial build of the full benchmark suite from the PDF spec; bug fixes, metric issues, UI evolution
- [[Untitled Chat]] (2026-02-20) — surfaced deep architectural failures invalidating the benchmark approach; triggered the pivot to behavioral metrics and hysteresis design
- [[Unreal Engine setup for networking library]] (2026-02-24) — four-interface architecture and standalone library design emerged here, using the benchmarks workspace as development environment
- [[Untitled Chat]] (2026-02-24) — clustering visualization refinement; interaction-likelihood metrics and hysteresis thresholds formalized
- [[Unreal Engine networking library setup]] (2026-02-24) — WSL vs Windows toolchain decision for `arcane-client-unreal` development
- [[Network library architecture review]] (2026-03-02) — comprehensive architecture review resolving all major design tensions; SpacetimeDB-as-game-logic-authority decision finalized