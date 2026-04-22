---
type: entity
tags: [rust, architecture, crate, traits, shared-types, no-io, foundation]
---

# arcane-core

## What It Is
`arcane-core` is the foundational crate of the Arcane Rust workspace, providing shared traits and types used across all other crates. It contains no I/O logic, making it a pure definition layer — the contract that the rest of the system is built against.

## Origin & Evolution
`arcane-core` emerged from the need for a clean separation between interface definitions and implementation in a multi-crate workspace. As Arcane grew into a distributed system with distinct concerns (spatial indexing, rules evaluation, server pooling, cluster infrastructure), a shared vocabulary of traits and types became necessary to avoid circular dependencies and coupling between implementation crates. By keeping `arcane-core` I/O-free, it can be depended on by any crate in the workspace without dragging in runtime or networking concerns.

## Technical Details
- **No I/O**: The crate deliberately excludes any I/O, networking, or async runtime dependencies, making it lightweight and universally composable within the workspace.
- **Traits and shared types**: Serves as the single source of truth for the interfaces that crates like `arcane-spatial`, `arcane-rules`, `arcane-pool`, and `arcane-infra` implement or consume.
- **Dependency direction**: All other crates in the workspace can depend on `arcane-core`; `arcane-core` depends on nothing internal to the workspace, enforcing a strict acyclic dependency graph.

## Key Design Decisions
- **No I/O in core** — Keeps the trait/type layer decoupled from runtime choices (tokio, async-std, etc.), allowing implementations to vary independently.
- **Separate from infra** — Infrastructure concerns (ClusterManager, replication, binaries) live in `arcane-infra`, not here, preventing the shared contract layer from becoming a grab-bag of implementation details.
- **Workspace-first design** — Structuring Arcane as a multi-crate workspace with `arcane-core` as the foundation was a deliberate architectural choice to enable independent development and testing of spatial, rules, pooling, and infra concerns.

## Relationships
- [[arcane-spatial]] — depends on `arcane-core` traits for its `SpatialIndex` implementation
- [[arcane-rules]] — depends on `arcane-core` for the `RulesEngine` interface
- [[arcane-pool]] — depends on `arcane-core` for `LocalPool` typing
- [[arcane-infra]] — depends on `arcane-core` for all shared types used in `ClusterManager`, `ClusterServer`, and replication logic
- [[arcane]] — the Rust workspace that houses `arcane-core` alongside all sibling crates

## Conversations That Shaped This
- [[Network library architecture review]]
- [[Claude Code session — pgp-demo]]