---
type: entity
tags: [rust, spatial, crate, grid, neighbor-discovery, indexing, arcane-workspace]
---

# arcane-spatial

## What It Is
`arcane-spatial` is a dedicated Rust crate in the Arcane workspace that provides a `SpatialIndex` — a 2D grid structure for neighbor discovery. It serves as the spatial reasoning layer of the backend, enabling ClusterServers and the clustering system to efficiently determine which entities, players, or zones are near one another without performing expensive exhaustive searches.

## Origin & Evolution
The crate emerged from the need to support large player counts across distributed ClusterServers, where naive broadcast or full-scan approaches to proximity checks would not scale. By isolating spatial indexing into its own crate (`arcane-spatial`), the Arcane workspace follows its general principle of separating concerns with no-I/O trait-and-type crates (`arcane-core`) and focused implementation crates. The 2D grid approach reflects a pragmatic choice suited to game worlds where planar proximity (rather than full 3D octree complexity) covers the dominant use cases — movement, physics neighborhoods, and interest management for replication.

## Technical Details
- Exposes a `SpatialIndex` type built on a 2D grid data structure.
- Designed for **neighbor discovery**: given an entity's position, efficiently retrieve all entities within a spatial neighborhood.
- Lives as an independent crate with no I/O dependencies, consistent with the workspace pattern of keeping logic crates pure and testable.
- Intended to be consumed by `arcane-infra` (ClusterManager, ClusterServer) and `arcane-rules` (RulesEngine) where clustering and replication decisions depend on spatial proximity.
- The grid-based approach provides O(1) or O(k) neighbor lookups (where k is neighbors found) versus O(n) linear scans over all entities.

## Key Design Decisions
- **2D grid over 3D structure** — covers the dominant game-world proximity use case without the overhead of octree or k-d tree implementations; can be revisited if 3D worlds become a target.
- **Isolated crate with no I/O** — keeps `arcane-spatial` pure and independently testable, matching the workspace-wide pattern established by `arcane-core`.
- **Neighbor discovery as the primary interface** — the API is scoped to answering "who is near this position?" rather than being a general geometry library, keeping the surface area small.
- **Separate from rules logic** — spatial queries are not mixed into `arcane-rules`; the RulesEngine consumes spatial results rather than owning the index, preserving separation of concerns.

## Relationships
- [[arcane-core]] — shared traits and types that `arcane-spatial` builds on
- [[arcane-rules]] — RulesEngine consumes spatial neighbor data to make clustering decisions
- [[arcane-infra]] — ClusterManager and ClusterServer use spatial indexing for replication scoping and entity interest management
- [[arcane-pool]] — LocalPool may interact with spatial data for server assignment
- [[ClusterServer]] — primary runtime consumer of neighbor discovery during simulation ticks
- [[ClusterManager]] — uses spatial context for cluster topology decisions

## Conversations That Shaped This
- [[Network library architecture review]]
- [[Claude Code session — e8dec835-2815-452e-81db-dbcda130475a]]