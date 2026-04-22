---
type: entity
tags: [architecture, rust, traits, server-pool, clustering, arcane-core, arcane-pool, interface]
---

# IServerPool

## What It Is
`IServerPool` is one of Arcane's four foundational interfaces, defined in `arcane-core` as a trait with no I/O. It abstracts the registry of game servers available to the cluster — tracking which servers exist, their capacities, and their current load — so that clustering and routing decisions can be made against a stable contract regardless of the underlying pool implementation.

## Origin & Evolution
`IServerPool` emerged from the original four-interface architectural pattern established in the very first design session (2026-02-24), which identified `IClusteringModel`, `IServerPool`, `IReplicationChannel`, and `IWorldSimulator` as the core abstraction seams of the library. The motivation was to keep the library standalone and engine-agnostic: by expressing server pool management as a trait, Arcane avoids coupling clustering logic to any particular deployment topology (local in-process pools, cloud-managed fleets, or mock pools for testing). The concrete reference implementation, `LocalPool`, lives in the `arcane-pool` crate, keeping I/O concerns out of `arcane-core`.

## Technical Details
- **Defined in:** `arcane-core` (traits and shared types, no I/O)
- **Reference implementation:** `arcane-pool` → `LocalPool`
- **Role in the pipeline:** `IServerPool` is consumed by the `RulesEngine` (`arcane-rules`) and `ClusterManager` (`arcane-infra`) to enumerate available servers and their state when making clustering decisions
- **No I/O constraint:** the trait itself carries no async runtime dependency, allowing it to be implemented by in-memory mocks in unit tests or by network-backed registries in production
- **Pluggability:** alternative implementations (e.g., a Kubernetes-aware pool, a cloud-fleet adapter) can be swapped in without changing the clustering or replication layers

## Key Design Decisions
- **Trait in `arcane-core`, impl in `arcane-pool`** — keeps the shared contract free of I/O and runtime dependencies, enabling pure-unit testing of clustering logic against mock pool implementations
- **Part of the four-interface seam** — grouping `IServerPool` alongside `IClusteringModel`, `IReplicationChannel`, and `IWorldSimulator` ensures every major axis of variability in the backend is behind an abstraction boundary from day one
- **`LocalPool` as reference, not default** — shipping a concrete `LocalPool` in `arcane-pool` gives users a working starting point without implying it is the only valid topology; fleet-scale operators are expected to provide their own implementation
- **No game-logic coupling** — `IServerPool` knows only about server presence and capacity, not about game state or entity ownership; game logic routing goes through SpacetimeDB reducers, keeping the pool interface narrow

## Relationships
- [[arcane-core]] — crate where `IServerPool` is defined as a trait
- [[arcane-pool]] — crate containing `LocalPool`, the reference implementation
- [[IClusteringModel]] — sibling interface; consults `IServerPool` when making clustering decisions
- [[IReplicationChannel]] — sibling interface in the four-interface design
- [[IWorldSimulator]] — sibling interface in the four-interface design
- [[RulesEngine]] — `arcane-rules` consumer that queries the pool to drive clustering decisions
- [[ClusterManager]] — `arcane-infra` component that uses pool state for server orchestration
- [[LocalPool]] — concrete struct implementing `IServerPool` for local/reference deployments

## Conversations That Shaped This
- [[Unreal Engine setup for networking library]] — introduced the four-interface design and established `IServerPool` as one of the core abstraction seams
- [[Network library architecture review]] — confirmed the trait/impl split across `arcane-core` and `arcane-pool`; reinforced the no-I/O constraint on core traits