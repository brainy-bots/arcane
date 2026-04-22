---
type: entity
tags: [clustering, infrastructure, rust, arcane-infra, coordination, websocket, redis, replication]
---

# ClusterManager

## What It Is
ClusterManager is the central coordination component of the Arcane multiplayer backend, living in the `arcane-infra` crate. It acts as the orchestrator for the cluster of game servers — handling player join routing, cluster topology decisions, and the lifecycle of ClusterServer instances. It is the authoritative entry point through which clients discover which server to connect to, and through which the system decides how to partition players across simulation nodes.

## Origin & Evolution
ClusterManager emerged from early PGP (Player Globe Partitioning) benchmark work in February 2026, where a dedicated HTTP coordination server was needed to route players and validate clustering strategy hypotheses. The original benchmark implementation exposed a fundamental architectural problem: naive spatial clustering caused servers to oscillate between merging and splitting due to proximity thresholds, and the benchmark itself had data integrity failures that undermined hypothesis validation.

This led to a significant architectural pivot: clustering decisions were grounded in **interaction-likelihood metrics** (guild membership, party relationships, enemy states) rather than raw spatial proximity, with hysteresis thresholds added to prevent oscillation. The four-interface design — `IClusteringModel`, `IServerPool`, `IReplicationChannel`, and `IWorldSimulator` — was introduced to keep ClusterManager's decisions pluggable and engine-agnostic.

By March 2026, a comprehensive architecture review resolved ten major design tensions, establishing that game logic belongs in SpacetimeDB reducers rather than ClusterServers. This clarified ClusterManager's role: it orchestrates topology and routing without owning game state, and does not need to mediate TCP RPC between servers for game actions.

## Technical Details
- Lives in `arcane-infra`; exposed as the `arcane-manager` binary (`cargo run -p arcane-infra --bin arcane-manager --features manager`)
- Exposes an **HTTP join endpoint** through which clients request cluster assignment before opening a WebSocket connection to the assigned ClusterServer
- Interfaces with `arcane-rules` (RulesEngine) for clustering decisions, `arcane-pool` (LocalPool) for server pool state, and `arcane-spatial` (SpatialIndex) for neighbor discovery inputs
- ClusterServers report state back (player counts, load signals) so ClusterManager can trigger server spawn or consolidation
- When players converge and load rises, ClusterManager spawns new servers rather than collapsing into a single overloaded node
- Does not own persistent game state — SpacetimeDB is the authoritative source; ClusterManager coordinates topology only
- Monitoring posture: Prometheus metrics exposed; intended to integrate with Grafana dashboards for cluster health visibility

## Key Design Decisions
- **HTTP for join, WebSocket for simulation** — ClusterManager uses HTTP coordination so clients can obtain a routing assignment cheaply before committing a long-lived WebSocket connection to a ClusterServer
- **Interaction-likelihood clustering, not spatial proximity** — clustering on who players are *likely* to interact with (social graph: guild, party, enemy) rather than where they stand, reducing cross-cluster communication and improving simulation locality
- **Hysteresis thresholds** — cluster merge/split decisions include hysteresis to prevent oscillation, a failure mode observed and diagnosed in the early benchmark implementations
- **Game logic in SpacetimeDB, not ClusterManager** — discrete game actions and persistent state are handled by SpacetimeDB reducers; ClusterManager and ClusterServers handle only high-frequency simulation routing, eliminating the need for inter-cluster TCP RPC for game events
- **Pluggable decision interfaces** — `IClusteringModel` is an interface, not a fixed implementation, allowing static rules today and ML-driven clustering later without changing the coordinator

## Relationships
- [[ClusterServer]] — the simulation nodes ClusterManager routes players to and whose lifecycle it manages
- [[RulesEngine]] — (`arcane-rules`) provides the clustering decision logic consumed by ClusterManager
- [[LocalPool]] — (`arcane-pool`) the server pool implementation ClusterManager draws from
- [[SpatialIndex]] — (`arcane-spatial`) 2D grid used as input signal for neighbor discovery
- [[SpacetimeDB]] — authoritative game state store; ClusterManager topology decisions do not override SpacetimeDB state ownership
- [[ReplicationChannel]] — ClusterServers replicate entity state; ClusterManager's routing determines which channel a client is subscribed to
- [[arcane-infra]] — the crate that contains ClusterManager

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]] — first full implementation of a PGP Cluster Manager with HTTP coordination; surfaced metric accuracy bugs and architectural gaps
- [[Untitled Chat (2026-02-20)]] — diagnosed benchmark failures and identified the deeper architectural invalidity of naive clustering approaches
- [[Untitled Chat (2026-02-24)]] — introduced interaction-likelihood metrics and hysteresis thresholds as the correct clustering basis
- [[Network library architecture review]] — resolved ten major design tensions; established game logic in SpacetimeDB and clarified ClusterManager's pure coordination role
- [[Project documentation overview]] — separation of `arcane-infra` as a pure clustering/replication library vs. demo concerns, affecting how ClusterManager is exposed as a general-purpose component