---
type: entity
tags: [pgp, partitioning, clustering, spatial-grid, social-affinity, architecture, arcane-core, benchmarking]
---

# Player Globe Partitioning

## What It Is
Player Globe Partitioning (PGP) is Arcane's core architectural strategy for distributing players across cluster servers. Rather than partitioning by raw geographic or spatial position alone, PGP clusters players by **social affinity** — grouping guild members, party members, and frequent collaborators onto the same cluster server — to minimize cross-cluster communication overhead for the interactions that matter most.

## Origin & Evolution
PGP emerged from the foundational problem Arcane was built to solve: enabling physics-accurate, combat-grade multiplayer simulation at player counts that exceed what any single dedicated game-engine server can handle. The key insight was that naive spatial partitioning causes excessive cross-cluster RPC traffic for socially-connected players who may be geographically dispersed. A formal PDF specification was produced to define the approach, which was then empirically validated through a purpose-built benchmark suite (`arcane-scaling-benchmarks`) in a focused session on 2026-02-20. That benchmark compared a spatial-grid baseline against the PGP Cluster Manager to measure whether social-affinity clustering produces measurably better cross-cluster communication characteristics — testing the hypothesis that drives the entire platform design.

## Technical Details
The PGP system involves several coordinated components:

- **PGP Cluster Manager**: An HTTP coordination layer that assigns players to cluster servers using a spatial index augmented with social-graph data. It consults affinity signals (guild, party membership) when making placement decisions.
- **Spatial Grid (arcane-spatial)**: A 2D grid `SpatialIndex` used for neighbor discovery. In the benchmark suite this served as the baseline comparison point; in production it informs proximity-aware placement within the social-affinity grouping.
- **Cluster Servers**: Individual server nodes running WebSocket-based player simulation. Cross-cluster interactions (e.g., attacks between players on different servers) are handled via TCP RPC. The benchmark measured the rate and latency of these cross-cluster RPC calls as the primary signal.
- **Load Generator**: A multi-mode tool that simulates player sessions and combat patterns to drive the benchmark scenarios.
- **Observability**: Prometheus metrics and Grafana dashboards were wired up to measure tick rate, cross-cluster RPC failure rates, and communication latency. A bug was found and fixed where the tick rate metric emitted at ~1 kHz instead of the intended 20 Hz.

The benchmark infrastructure also exposed and fixed several implementation bugs: parameter ordering in the spatial index, incorrect RPC failure reporting, and Docker networking issues for service discovery.

## Key Design Decisions
- **Social affinity over pure spatial partitioning** — Socially-connected players generate far more inter-player events than random spatial neighbors; co-locating them on the same cluster server converts expensive cross-cluster RPCs into cheap in-process calls.
- **HTTP coordination layer for cluster assignment** — The PGP Cluster Manager uses HTTP so placement decisions can be made at join-time and adjusted without coupling to the WebSocket simulation path.
- **TCP RPC for cross-cluster communication** — Cross-cluster attacks and events use TCP RPC rather than routing through Redis or a message bus, keeping latency predictable and the path explicit.
- **Empirical validation before production commitment** — The specification was implemented as a benchmark suite first, with a Spatial Grid baseline as a controlled comparison, before the hypothesis was treated as confirmed.
- **Prometheus + Grafana as first-class observability** — Metrics were instrumented from the start of the benchmark, making it possible to catch subtle bugs (e.g., tick rate emission frequency) that would have invalidated results.

## Relationships
- [[SpatialIndex]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[RulesEngine]]
- [[LocalPool]]
- [[arcane-spatial]]
- [[arcane-infra]]
- [[arcane-core]]
- [[Cross-Cluster RPC]]
- [[Redis Replication]]

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]]