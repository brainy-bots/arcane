---
type: entity
tags: [interface, trait, architecture, simulation, game-logic, arcane-core, design-pattern]
---

# IWorldSimulator

## What It Is
`IWorldSimulator` is one of the four core trait interfaces in Arcane's `arcane-core` crate, defining the contract for world simulation logic — physics ticks, movement, AI updates, and high-frequency entity state. It acts as the seam between Arcane's infrastructure layer (ClusterServers, replication, clustering) and game-specific simulation code, keeping the backend engine-agnostic and the simulation logic pluggable.

## Origin & Evolution
`IWorldSimulator` was conceived in the earliest architecture session as part of the foundational four-interface design pattern. When the project's direction was clarified — that Arcane should be a *standalone library* with Unreal as just one possible client, not a tightly-coupled Unreal plugin — the team needed clean abstraction boundaries. Rather than baking any specific engine's simulation assumptions into the backend, `IWorldSimulator` was introduced alongside `IClusteringModel`, `IServerPool`, and `IReplicationChannel` to let each concern be implemented independently and swapped out. The key milestone was the architecture review session (2026-03-02), which resolved where game logic actually lives: high-frequency simulation (movement, physics, AI ticks) runs in ClusterServer processes behind `IWorldSimulator`, while discrete game actions and persistent state live in SpacetimeDB reducers. This split eliminated the need for TCP RPC between clusters for game actions and gave `IWorldSimulator` a sharply scoped responsibility.

## Technical Details
`IWorldSimulator` is defined in `arcane-core` (no I/O, traits and shared types only), ensuring it carries no infrastructure dependencies. Implementors are expected to run inside a ClusterServer process, consuming the high-frequency simulation tick budget — typically physics, movement integration, and AI state updates for entities owned by that cluster node. ClusterServers call into the simulator, collect owned entity state diffs, and hand those diffs to `IReplicationChannel` for broadcast. The interface is intentionally narrow: it does not own network concerns, persistence, or clustering decisions. Those belong to the other three core interfaces. Pluggable implementations allow a studio to drop in a physics engine, a deterministic simulation library, or a test stub without touching infrastructure code.

## Key Design Decisions
- **Defined in `arcane-core` with no I/O** — keeps the trait usable across crates and test harnesses without pulling in networking or storage dependencies
- **Scoped to high-frequency simulation only** — discrete game actions (combat resolution, loot, progression) are SpacetimeDB reducers, not simulator responsibility; this avoids ambiguous ownership and eliminates cross-cluster RPC for game logic
- **Part of the four-interface pattern from day one** — `IClusteringModel`, `IServerPool`, `IReplicationChannel`, and `IWorldSimulator` were introduced together as the abstraction strategy that keeps Arcane engine-agnostic
- **Implementations live in ClusterServer processes** — simulation runs close to the network edge, not in a manager or persistence layer, matching the data-locality requirements of physics at scale

## Relationships
- [[IClusteringModel]] — sibling interface; decides which server owns which entities
- [[IServerPool]] — sibling interface; manages the pool of available ClusterServer instances
- [[IReplicationChannel]] — sibling interface; consumes state diffs produced by the simulator and broadcasts them
- [[arcane-core]] — the crate where `IWorldSimulator` is defined
- [[ClusterServer]] — the runtime host that calls `IWorldSimulator` implementations
- [[SpacetimeDB]] — counterpart for persistent/discrete game logic that `IWorldSimulator` explicitly does *not* own

## Conversations That Shaped This
- [[Unreal Engine setup for networking library]] — session where the four-interface design pattern was first articulated and `IWorldSimulator` was named
- [[Network library architecture review]] — session where the simulation/persistence split was resolved, sharpening `IWorldSimulator`'s scope to high-frequency ticks only