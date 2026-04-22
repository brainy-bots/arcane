---
type: entity
tags: [spatial, arcane-spatial, neighbor-discovery, clustering, rust, grid, indexing]
---

# Spatial Grid

## What It Is
The Spatial Grid is a 2D grid-based spatial index living in the `arcane-spatial` crate, exposed as the `SpatialIndex` type. It is the core data structure Arcane uses for neighbor discovery — determining which players or entities are near enough to one another to require replication, interest management, or clustering decisions.

## Origin & Evolution
The Spatial Grid first appeared as a concrete implementation artifact during the PGP benchmark effort (February 2026), where a dedicated "Spatial Grid Server" was built as the **baseline comparator** against the Player Globe Partitioning clustering strategy. The goal was to empirically demonstrate whether social-affinity clustering outperformed naive spatial binning for cross-cluster communication characteristics. A parameter-ordering bug in the spatial index was among the first bugs surfaced and fixed during that benchmark work. As Arcane matured from a benchmark scaffolding into a proper library, the spatial index was promoted to its own first-class crate (`arcane-spatial`) within the workspace, separating it cleanly from I/O concerns.

## Technical Details
- Lives in the `arcane-spatial` crate, isolated from I/O (no async, no network code).
- Implements a 2D grid: the game world is divided into cells; entities are bucketed by cell coordinates.
- Primary operation is **neighbor lookup**: given an entity's position, return all entities in adjacent cells within some radius or cell distance.
- Used upstream by `arcane-rules` (the `RulesEngine`) to make clustering decisions, and by `arcane-infra`'s `ClusterServer`/`ClusterManager` to drive replication topology.
- The `SpatialIndex` trait is defined in `arcane-core` (traits and shared types, no I/O); `arcane-spatial` provides the concrete grid implementation.
- Serialization is done once per tick in the broadcast-first replication model; the spatial grid feeds the entity set that gets packed into `EntityStateDelta` before broadcast.

## Key Design Decisions
- **Separate crate with no I/O** — keeps the index purely functional and easily testable without spinning up servers or async runtimes.
- **Grid cells over k-d trees or BVH** — simpler to implement and update at game-server tick rates where entities move every frame; insert/remove is O(1) per entity.
- **Baseline for benchmarking** — deliberately positioned as the "dumb spatial" comparator so PGP's social-affinity clustering could be measured against a known reference.
- **Trait in core, implementation in spatial** — follows Arcane's pattern of defining interfaces in `arcane-core` and keeping concrete implementations in domain crates, enabling alternative spatial backends without touching upstream crates.

## Relationships
- [[arcane-spatial]] — the crate that owns this implementation
- [[arcane-core]] — defines the `SpatialIndex` trait
- [[arcane-rules]] — consumes spatial queries to drive clustering decisions
- [[ClusterServer]] — uses neighbor data to determine which entity states to replicate
- [[ClusterManager]] — uses spatial data to assign players to clusters
- [[RulesEngine]] — the clustering decision layer that sits on top of spatial queries
- [[PGP Clustering]] — the social-affinity strategy benchmarked against the spatial grid as baseline
- [[EntityStateDelta]] — the replication payload built from spatially-scoped entity sets

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]] — spatial grid server built as PGP benchmark baseline; parameter-ordering bug found and fixed
- [[Untitled Chat]] — grid server flatlined at 0 Hz, exposing metric and architectural issues with the baseline approach
- [[Network library architecture review]] — spatial index promoted to first-class `arcane-spatial` crate in the library restructure
- [[STATE_UPDATE message handling in ClusterServer]] — traces how spatial neighbor data flows into the broadcast replication pipeline
- [[Standalone binary for Unreal Engine testing]] — spatial index used as part of the full-stack benchmark comparing Arcane vs SpacetimeDB at scale