---
type: entity
tags: [pgp-demo, benchmarking, clustering, spatial-grid, websocket, redis, prometheus, grafana, docker, rust, load-testing, hypothesis-testing, visualization]
---

# pgp-demo

## What It Is
`pgp-demo` is a standalone benchmark and demonstration suite for the **Player Globe Partitioning** (PGP) clustering strategy — Arcane's core hypothesis that clustering players by social affinity (guilds, parties, enemy relationships) produces better cross-cluster communication characteristics than clustering by raw spatial position. It lives in the `arcane-scaling-benchmarks` repository and serves as the empirical validation layer for Arcane's architectural bets before they are baked into the production library.

## Origin & Evolution
The demo originated from a PDF specification (2026-02-20) that laid out a structured benchmark plan for testing PGP against a naive spatial-grid baseline. The first session produced a working multi-component stack: a Spatial Grid Server for baseline comparison, a PGP Cluster Manager with HTTP coordination and spatial indexing, individual Cluster Servers running WebSocket simulation with TCP RPC for cross-cluster attacks, a multi-mode Load Generator, and a Prometheus + Grafana monitoring stack.

The initial implementation immediately revealed problems. Despite a hardcoded 20-player-per-server limit, 1,000+ players were being assigned to a single server, making the benchmark results meaningless. Metric accuracy was also broken — the cluster server reported a consistent 1 kHz tick rate while the grid server flatlined at 0 Hz, and most metrics remained at zero despite the system running. UI controls for dynamic parameter adjustment (active player count, interactions per player, cluster configuration) reset to zero without effect, making continuous parameter-space exploration impossible.

These failures triggered a pivot from a fully automated benchmark toward a behavior-simulation approach grounded in **interaction-likelihood metrics** rather than position proximity. The visualization layer was rebuilt to reflect who players are *likely to interact with* (guild, party, enemy state), and hysteresis thresholds were introduced to prevent the merge-split oscillation instability in naive threshold-based systems. Server load was integrated as a scaling signal: when players converge, the system spawns new servers rather than collapsing an overloaded cluster.

By 2026-03-02, the broader Arcane architecture review clarified that game logic belongs in SpacetimeDB reducers rather than ClusterServers, which retroactively invalidated the TCP RPC between clusters for game actions that the early pgp-demo had used. This simplified the conceptual model the demo was trying to validate.

## Technical Details
The benchmark stack consists of:

- **Spatial Grid Server** — baseline comparison node using a 2D grid for neighbor discovery, corresponding to `arcane-spatial`'s `SpatialIndex`
- **PGP Cluster Manager** — HTTP coordination layer with spatial indexing; assigns players to clusters based on affinity scores rather than grid cells
- **Cluster Servers** — WebSocket simulation nodes; early versions used TCP RPC for cross-cluster attack propagation (later reconsidered in favor of SpacetimeDB-mediated state)
- **Load Generator** — multi-mode synthetic player traffic; requires manual triggering of player creation and interaction events to produce meaningful dashboard output
- **Prometheus + Grafana stack** — observability layer; exposed metric accuracy bugs (1 kHz tick rate emission instead of intended 20 Hz) and flat-zero counters that required Docker networking fixes for service discovery

Key bugs fixed during development:
- Parameter ordering error in the spatial index
- Incorrect RPC failure reporting inflating cross-cluster error metrics
- Docker network isolation preventing service discovery between Grafana and Prometheus targets
- Tick rate metric emitting at ~1 kHz instead of the designed 20 Hz

The Unreal client work that ran in parallel (sessions 2026-02-24) established that Arcane's client layer would be engine-agnostic — `arcane-client-unreal` replaces Unreal's native replication rather than layering on top of it — which influenced what the demo needed to prove: the backend clustering strategy must be independently defensible without assuming any client engine.

## Key Design Decisions
- **Social affinity over spatial proximity** — The core PGP hypothesis: guild/party/enemy relationships are better predictors of cross-cluster communication load than grid position. The demo exists to validate this empirically.
- **Hysteresis thresholds for cluster stability** — Naive threshold-based clustering oscillates (merge → split → merge). Hysteresis was added to require a sustained condition before a clustering decision is committed.
- **Server load as a scaling signal** — When players converge, new servers spawn rather than overloading the existing cluster; this mirrors the `arcane-rules` RulesEngine design.
- **Manual trigger model for load generation** — Continuous auto-benchmarking proved unworkable given metric accuracy problems; the demo shifted to manually triggered scenarios with observable Grafana output.
- **TCP RPC between clusters (later abandoned)** — Early cross-cluster attack propagation used direct TCP RPC. The 2026-03-02 architecture review concluded game actions should route through SpacetimeDB reducers instead, making this unnecessary.
- **Standalone benchmark repo** — Kept separate from the main `arcane` workspace to avoid polluting the production library with heavy Docker/monitoring tooling and to allow the benchmark to evolve independently.

## Relationships
- [[arcane-spatial]] — `SpatialIndex` used in both the baseline grid server and the PGP Cluster Manager
- [[arcane-rules]] — `RulesEngine` design reflects the same clustering-decision logic the demo validates
- [[arcane-infra]] — `ClusterManager` and `ClusterServer` production implementations correspond to the demo's analogous components
- [[arcane-cluster]] — WebSocket cluster binary the demo's Cluster Servers prefigure
- [[arcane-manager]] — HTTP manager binary the demo's PGP Cluster Manager prefigures
- [[SpacetimeDB]] — Identified during architecture review as the correct home for game actions that the demo was routing through TCP RPC
- [[arcane-client-unreal]] — Parallel development thread; established the engine-agnostic client constraint the backend must satisfy

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]] (2026-02-20) — Main build session; produced the working stack and fixed core bugs
- [[Untitled Chat]] (2026-02-20) — Identified metric accuracy failures and architectural invalidity of the initial approach; triggered the behavioral-metrics pivot
- [[Untitled Chat]] (2026-02-24) — Rebuilt visualization around interaction-likelihood metrics; introduced hysteresis and server-load-as-scaling-signal
- [[Network library architecture review]] (2026-03-02) — Resolved the TCP RPC question; established SpacetimeDB as authoritative for game actions, retroactively simplifying what pgp-demo needed to prove