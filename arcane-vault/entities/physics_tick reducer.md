---
type: entity
tags: [arcane, spacetimedb, reducer, physics, simulation, tick-rate, benchmarking, rust]
---

# physics_tick reducer

## What It Is
`physics_tick` is a SpacetimeDB scheduled reducer that drives server-side physics simulation on a fixed interval (10 Hz by default). It is the core simulation loop when SpacetimeDB is used as Arcane's backend, responsible for advancing entity state — positions, velocities, collisions — for all players each tick. It represents the upper-bound constraint on SpacetimeDB's solo scalability within the Arcane architecture.

## Origin & Evolution
The reducer emerged as part of the effort to establish a fair, apples-to-apples comparison between a pure SpacetimeDB backend and Arcane's distributed cluster architecture. Earlier benchmarking approaches used HTTP REST polling to simulate clients, which unfairly penalized SpacetimeDB; the team moved to a headless Rust swarm binary (`arcane-swarm`) using the real SpacetimeDB SDK (WebSocket + BSATN + subscriptions). As part of locking down canonical workload parameters (10 Hz tick rate, 2 actions/sec, 30-second runs, everyone-sees-everyone visibility), `physics_tick` was identified as the server-side scheduled reducer doing the actual simulation work. Benchmarking revealed that with `physics_tick` running, the SpacetimeDB-only ceiling is approximately 1,000 concurrent players — the key data point used to argue for Arcane's distributed approach beyond that scale.

## Technical Details
- **Execution model:** Scheduled reducer in SpacetimeDB, invoked on a fixed interval (10 Hz = 100 ms per tick).
- **Responsibility:** Advances physics state (positions, velocities, collision resolution) for all entities in the table each invocation.
- **Workload characteristics used in benchmarks:** 10 Hz tick rate, 2 client actions/sec, spread movement pattern, full visibility (everyone-sees-everyone).
- **Scalability ceiling:** ~1,000 concurrent players on a single SpacetimeDB instance with `physics_tick` active; beyond this, the single-process nature of SpacetimeDB becomes the bottleneck.
- **Comparison point:** Arcane's cluster architecture distributes this simulation work across multiple `ClusterServer` nodes, each handling a spatial partition, allowing the ceiling to be broken horizontally.

## Key Design Decisions
- **10 Hz tick rate** — Chosen as the canonical benchmark frequency; represents a realistic game server update rate balancing simulation fidelity and throughput.
- **Server-side physics in SpacetimeDB** — Moving physics into the reducer (rather than trusting client-reported positions) was necessary for authoritative simulation, but concentrates CPU load in a single WASM process.
- **Used as the SpacetimeDB scalability ceiling marker** — The reducer's performance under load (not WebSocket throughput or subscription fan-out) is what caps the ~1,000 player ceiling, making it the central evidence for why Arcane's distributed architecture is necessary at larger scale.

## Relationships
- [[arcane-swarm]] — headless Rust binary that simulates real SpacetimeDB clients against the reducer
- [[SpacetimeDB]] — the runtime that hosts and schedules `physics_tick`
- [[ClusterServer]] — Arcane's distributed alternative to centralised `physics_tick` simulation
- [[ClusterManager]] — orchestrates which ClusterServers handle which spatial partitions
- [[arcane-spatial]] — SpatialIndex used in Arcane's distributed physics partitioning
- [[benchmarking methodology]] — canonical workload parameters that frame the 1,000-player ceiling finding

## Conversations That Shaped This
- [[Standalone binary for Unreal Engine testing]] (2026-03-06) — session where `physics_tick` was identified as the SpacetimeDB scalability ceiling, canonical benchmark parameters were locked, and the ~1,000 concurrent player limit was established