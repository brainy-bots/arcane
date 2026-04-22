---
type: entity
tags: [clustering, affinity, spatial, pgp, rules-engine, architecture, benchmarking]
---

# Affinity Clustering

## What It Is
Affinity Clustering is the core architectural hypothesis of Arcane: that players should be grouped onto cluster servers based on *who they are likely to interact with* — guild membership, party relationships, enemy states — rather than raw spatial proximity. In Arcane, the [[RulesEngine]] in `arcane-rules` encodes these clustering decisions, and the [[ClusterManager]] executes them, ensuring that cross-cluster communication (the most expensive operation) is minimized by co-locating players who will actually interact.

## Origin & Evolution
The concept originated as the central question of the PGP (Player Globe Partitioning) benchmark project: does social-affinity clustering produce better cross-cluster communication characteristics than spatial-grid clustering? An entire benchmark suite was built to test this empirically — a Spatial Grid Server as baseline, a PGP Cluster Manager with social-affinity logic, and a Prometheus/Grafana monitoring stack to compare them. The benchmarking effort exposed significant implementation difficulties (metrics flatlined at zero, UI controls resetting without effect, 1 kHz tick-rate misreporting), which eroded confidence in the benchmark's validity before it could produce clean results.

The visualization phase that followed deepened the concept: a static single-cluster demo was replaced with a behavioral simulation grounded in interaction-likelihood metrics. This is where two key refinements were codified — **hysteresis thresholds** (to prevent oscillation between merge and split) and **server load as a scaling signal** (spawn new servers when players converge, rather than collapsing into an overloaded cluster). By the full architecture review session, affinity clustering was treated as settled design: the [[RulesEngine]] makes clustering decisions, [[ClusterServer]] executes simulation, and cross-cluster attack or interaction traffic is routed through [[SpacetimeDB]] reducers rather than direct TCP RPC, eliminating much of the inter-cluster overhead that affinity clustering was designed to reduce.

## Technical Details
The clustering decision pipeline runs through `arcane-rules` (`RulesEngine`), which consumes player state — including social graph signals (guild, party, enemy flags) and spatial data from `arcane-spatial` (`SpatialIndex`, a 2D grid) — and emits clustering assignments. The [[ClusterManager]] in `arcane-infra` receives these assignments and routes players to [[ClusterServer]] instances accordingly.

Key structural properties:
- **Interaction-likelihood as the primary signal**: guild/party membership and active enemy relationships outrank raw position when determining co-location.
- **Hysteresis thresholds**: merge/split decisions require the affinity signal to cross a threshold by a defined margin before acting, preventing rapid oscillation in dynamic player groups.
- **Server load integration**: cluster capacity pressure triggers server spawning rather than over-packing existing clusters; this is managed via the [[LocalPool]] in `arcane-pool`.
- **Spatial index as secondary signal**: `SpatialIndex` still informs neighbor discovery and proximity-based visibility, but is subordinate to social affinity for cluster assignment decisions.
- **SpacetimeDB for discrete actions**: cross-cluster game actions (attacks, trades) go through SpacetimeDB reducers, not cluster-to-cluster RPC, which decouples the clustering topology from game-action routing.

## Key Design Decisions
- **Social affinity over spatial proximity** — minimizes cross-cluster communication for the interactions that matter most (party play, guild combat), accepting that players may be on different clusters from nearby strangers
- **Hysteresis on merge/split thresholds** — prevents the known instability where clusters oscillate on every tick when near a threshold boundary
- **Server load as a co-equal scaling signal** — prevents clustering decisions from creating overloaded servers; capacity triggers spawn rather than re-pack
- **SpacetimeDB handles persistent game actions** — eliminates the need for TCP RPC between ClusterServers for discrete game events, a major cross-cluster traffic category
- **Spatial grid retained for neighbor discovery** — `SpatialIndex` still powers proximity queries (replication visibility, nearby-entity detection) even though it's not the primary clustering criterion

## Relationships
- [[RulesEngine]] — the `arcane-rules` crate that encodes and executes clustering decisions
- [[ClusterManager]] — orchestrates cluster assignment based on RulesEngine output
- [[ClusterServer]] — the simulation unit players are assigned to
- [[SpatialIndex]] — 2D grid used for proximity/neighbor signals, secondary to affinity
- [[LocalPool]] — manages server capacity; receives spawn signals when affinity clustering triggers scale-out
- [[SpacetimeDB]] — handles persistent game actions cross-cluster, reducing inter-cluster RPC load
- [[PGP Benchmark]] — the empirical validation project that tested affinity vs. spatial clustering
- [[Replication]] — broadcast model that delivers state to clients after clustering determines server assignment

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]] — built the first affinity vs. spatial benchmark suite; surfaced metric and UI reliability problems
- [[Untitled Chat (2026-02-20)]] — identified the benchmark's architectural failures and validity concerns
- [[Untitled Chat (2026-02-24)]] — formalized interaction-likelihood as the clustering signal; introduced hysteresis and server-load scaling
- [[Network library architecture review]] — settled affinity clustering as production design; resolved SpacetimeDB-for-game-actions decision that eliminates cross-cluster RPC
- [[STATE_UPDATE message handling in ClusterServer]] — examined replication model post-clustering; confirmed broadcast-first architecture that clustering assignments feed into