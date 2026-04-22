---
type: entity
tags: [crate, pool, server-management, rust, arcane-infra, local-pool]
---

# arcane-pool

## What It Is
`arcane-pool` is a dedicated Rust crate within the Arcane workspace that implements `LocalPool` — the server pool abstraction responsible for tracking and managing the collection of available cluster servers. It sits between the clustering decision logic and the infrastructure layer, providing a concrete pool implementation that the `arcane-infra` crate consumes when managing server assignments and load distribution.

## Origin & Evolution
The crate emerged as part of the deliberate separation of concerns across the Arcane workspace. Rather than bundling pool management directly into `arcane-infra` or `arcane-core`, it was factored out so that pool logic could evolve independently and remain testable without pulling in I/O or infrastructure dependencies. The architecture review session in March 2026 established that clustering decisions (owned by `arcane-rules`) and the pool of servers available to act on those decisions (owned by `arcane-pool`) should be separate concerns, with `arcane-infra`'s `ClusterManager` composing them together.

## Technical Details
The crate exposes `LocalPool`, a concrete implementation of the server pool interface defined in `arcane-core`. `LocalPool` maintains in-process state about available `ClusterServer` instances — their identities, load signals, and availability — without performing any I/O itself. The `arcane-infra` crate consumes `LocalPool` inside `ClusterManager`, which uses it alongside `arcane-rules`' `RulesEngine` and `arcane-spatial`'s `SpatialIndex` to route players to appropriate servers. Because the crate carries no I/O, it is independently unit-testable and has a minimal dependency footprint relative to the rest of the workspace.

## Key Design Decisions
- **Separated from `arcane-infra`** — keeps pool tracking logic independently testable and avoids coupling lifecycle management to transport or Redis concerns
- **No I/O in the crate** — consistent with the workspace-wide pattern (also followed by `arcane-core`, `arcane-spatial`, `arcane-rules`) of keeping infrastructure-free crates pure so they can be tested without network or storage dependencies
- **Implements traits from `arcane-core`** — pool behavior is defined by shared traits, allowing `arcane-infra` to depend on the abstraction and swap implementations without touching call sites

## Relationships
- [[arcane-core]] — defines the traits `LocalPool` implements
- [[arcane-infra]] — primary consumer; `ClusterManager` composes `LocalPool` with `RulesEngine` and `SpatialIndex`
- [[arcane-rules]] — peer crate; provides clustering decisions that operate on the pool
- [[arcane-spatial]] — peer crate; provides spatial indexing used alongside pool state for neighbor-aware server assignment
- [[ClusterManager]] — the runtime component that owns and drives a `LocalPool` instance

## Conversations That Shaped This
- [[Network library architecture review]]
- [[Claude Code session — pgp-demo]]