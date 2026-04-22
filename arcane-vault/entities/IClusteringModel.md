---
type: entity
tags: [architecture, rust, traits, clustering, arcane-core, interfaces, design-pattern]
---

# IClusteringModel

## What It Is
`IClusteringModel` is a core trait interface in the Arcane multiplayer backend library that defines how clustering decisions are made — determining which players/entities belong to which cluster servers. It is one of the four foundational interfaces of the Arcane architecture (alongside `IServerPool`, `IReplicationChannel`, and `IWorldSimulator`), living in the `arcane-core` crate to remain dependency-free and pluggable.

## Origin & Evolution
The interface emerged in the initial architectural session (2026-02-24) when the project pivoted away from an Unreal-plugin-first approach toward a standalone, engine-agnostic backend library. The four-interface design was conceived specifically to allow pluggable implementations — the clustering model slot was designed from the start to accommodate both simple rule-based implementations (`StaticRules`) and future ML-driven approaches. By the architecture review session (2026-03-02), `IClusteringModel` was implemented concretely in the `arcane-rules` crate as `RulesEngine`, which provides the clustering decision logic consumed by `ClusterManager` in `arcane-infra`.

## Technical Details
- Defined in **`arcane-core`** — the traits-and-shared-types crate with no I/O dependencies, ensuring the interface is portable across all crates
- Consumed by **`ClusterManager`** in `arcane-infra` to make runtime decisions about player-to-cluster assignment
- The primary concrete implementation is **`RulesEngine`** in the `arcane-rules` crate
- The interface is intentionally abstract to support two implementation families: static/rule-based clustering (spatial proximity, player count thresholds) and future ML-driven dynamic clustering
- Clustering decisions feed into the broader system flow: clients join via the HTTP manager binary (`arcane-manager`), which uses the clustering model to route them to the appropriate `ClusterServer` WebSocket endpoint

## Key Design Decisions
- **Placed in `arcane-core`, not `arcane-rules`** — separating the trait from its implementation means any crate can depend on the interface without pulling in rules-engine logic
- **Pluggable by design** — the slot was reserved for ML-driven models from day one, even though `StaticRules`/`RulesEngine` is the initial implementation; this avoided coupling the architecture to a single strategy
- **Clustering is a manager-level concern** — the 2026-03-02 review confirmed that clustering decisions happen in `ClusterManager`, not inside `ClusterServer`; this keeps simulation servers focused on high-frequency tick work (physics, movement, AI) rather than topology management
- **Decoupled from game logic** — game logic lives in SpacetimeDB reducers; `IClusteringModel` is purely a routing/assignment mechanism, not a game-state authority

## Relationships
- [[RulesEngine]] — concrete implementation in `arcane-rules`
- [[ClusterManager]] — primary consumer of the clustering model at runtime
- [[IServerPool]] — sibling interface; manages the pool of available servers that clustering decisions route into
- [[IReplicationChannel]] — sibling interface for state propagation between clusters
- [[IWorldSimulator]] — sibling interface for simulation execution within a cluster
- [[arcane-core]] — home crate for this trait
- [[arcane-rules]] — crate containing `RulesEngine`, the `StaticRules` implementation

## Conversations That Shaped This
- [[Unreal Engine setup for networking library]] — introduced the four-interface design pattern and the `IClusteringModel` slot
- [[Network library architecture review]] — resolved where clustering decisions live in the system (manager layer), confirmed `RulesEngine` as the concrete implementation, and established the separation between clustering topology and game logic