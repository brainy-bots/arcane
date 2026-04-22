---
type: entity
tags: [pgp, clustering, architecture, spatial-grid, social-affinity, benchmarking, cluster-server, partitioning]
---

# PGP Architecture

## What It Is
PGP (Player Globe Partitioning) is Arcane's core clustering strategy that partitions players across ClusterServers using social affinity (guilds, parties, social graphs) rather than raw spatial position. It serves as the foundational architectural hypothesis of the Arcane platform: that social-affinity-based clustering produces superior cross-cluster communication characteristics compared to purely geographic partitioning, enabling the system to scale well beyond what single-process game-engine servers can achieve.

## Origin & Evolution
PGP emerged from the core problem Arcane was built to solve: dedicated game-engine servers are fundamentally single-process, capping player counts by what one machine can simulate. A naive distributed answer — partition by geography — still generates heavy cross-cluster traffic whenever socially-linked players (party members, guild groups) end up on different servers. PGP was formulated as an alternative hypothesis: clustering by social relationship minimizes cross-cluster RPC calls because players who interact frequently are co-located on the same ClusterServer.

The benchmark suite built in the February 2026 sessions was a direct attempt to empirically validate this hypothesis. A Spatial Grid Server was constructed as the baseline comparison point, and a PGP Cluster Manager was built alongside it to run controlled experiments measuring cross-cluster communication overhead under each strategy. Early implementation work surfaced significant bugs (parameter ordering in the spatial index, RPC failure reporting, a tick rate metric emitting at ~1 kHz instead of the intended 20 Hz) and deeper architectural invalidations — notably that a hardcoded 20-player-per-server limit and lack of authoritative physics made the benchmark results unreliable as evidence for the hypothesis.

By the March 2026 architecture review sessions, PGP had evolved from a standalone benchmark concept into the clustering layer of a fully specified distributed system. The architecture was stabilized: ClusterServers own high-frequency simulation (movement, physics, AI ticks); SpacetimeDB holds persistent game state and discrete game actions; Redis handles inter-cluster state propagation. This eliminated the need for TCP RPC between clusters for game actions, which had been a significant complexity and performance concern in the earlier PGP demo design.

## Technical Details
The PGP architecture operates as follows: a **PGP Cluster Manager** (HTTP coordination layer) accepts join requests and uses a spatial index alongside social-graph metadata to assign players to ClusterServers. The `arcane-rules` crate encapsulates the `RulesEngine` that makes clustering decisions, keeping policy separate from infrastructure. The `arcane-spatial` crate provides the `SpatialIndex` (2D grid) used for neighbor discovery, which feeds into both the spatial baseline and the PGP strategy.

Each ClusterServer runs a WebSocket simulation tick loop. In the current replication model, ClusterServer uses a **broadcast-first, serialize-once** pattern: each tick it constructs an `EntityStateDelta`, merges neighbor cluster data in `cluster_runner`, and pushes it once over an mpsc channel to the WebSocket server, which serializes to JSON once and drops it into a tokio broadcast channel for all connected clients. Cross-cluster state sharing happens via Redis rather than direct TCP RPC between ClusterServers.

The benchmarking methodology that was ultimately locked down for fair comparison used: 10 Hz tick rate, 2 actions/sec per player, 30-second runs, spread movement, everyone-sees-everyone visibility, and a headless Rust `arcane-swarm` binary using the actual SpacetimeDB SDK (WebSocket + BSATN + subscriptions) rather than HTTP REST polling, which had previously biased results against SpacetimeDB.

## Key Design Decisions
- **Social affinity over spatial locality** — minimizes cross-cluster RPC by co-locating players who interact frequently; the central hypothesis distinguishing PGP from naive geographic partitioning
- **RulesEngine in `arcane-rules` crate** — clustering policy is isolated from infrastructure so it can be swapped, tested, and evolved independently
- **Redis for cross-cluster state, not TCP RPC** — eliminates direct server-to-server coupling for game actions; cross-cluster visibility handled through a shared pub/sub layer
- **SpacetimeDB for persistence, ClusterServer for simulation** — game logic (discrete actions, persistent state) lives in SpacetimeDB reducers; ClusterServers own only high-frequency ephemeral simulation state
- **Broadcast-first replication** — serialize once per tick, fan out identically to all clients; trades per-client filtering efficiency for implementation simplicity in the current model
- **Authoritative physics required in benchmarks** — the February 2026 session identified that toy physics invalidated benchmark results; real simulation load is necessary to measure PGP's advantage meaningfully
- **Four-bucket data model (Spine/Replicated/Ephemeral/Persistent)** — adopted over per-property replication flags to reduce wire overhead and make replication rules explicit at the type level

## Relationships
- [[ClusterServer]]
- [[ClusterManager]]
- [[RulesEngine]]
- [[SpatialIndex]]
- [[arcane-rules]]
- [[arcane-spatial]]
- [[arcane-infra]]
- [[SpacetimeDB Integration]]
- [[Redis Replication]]
- [[EntityStateDelta]]
- [[Four-Bucket Data Model]]
- [[arcane-swarm]]

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]] (2026-02-20) — initial PGP benchmark build from PDF spec; surfaced core bugs and hypothesis framing
- [[Untitled Chat]] (2026-02-20) — identified architectural failures in the benchmark (toy physics, hardcoded limits, broken UI controls) that invalidated early results
- [[Network library architecture review]] (2026-03-02) — resolved fundamental design tensions; established SpacetimeDB-for-persistence, ClusterServer-for-simulation split; eliminated TCP RPC between clusters
- [[Untitled Chat]] (2026-03-03) — formalized four-bucket data classification model; identified authoritative physics gap
- [[Standalone binary for Unreal Engine testing]] (2026-03-06) — locked down fair benchmarking methodology; established SpacetimeDB ceiling (~1000 players) vs. Arcane at scale
- [[STATE_UPDATE message handling in ClusterServer]] (2026-03-16) — traced full replication data path; documented broadcast-first serialize-once pattern and identified future per-client filtering insertion points