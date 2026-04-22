---
type: entity
tags: [benchmarking, rust, swarm, spacetimedb, arcane, load-testing, distributed-systems, performance]
---

# arcane-benchmark-swarm

## What It Is
The `arcane-swarm` is a headless Rust binary that simulates real game clients at scale for benchmarking the Arcane distributed cluster architecture against a SpacetimeDB-only backend. It uses the actual SpacetimeDB SDK (WebSocket + BSATN + subscriptions) to generate authentic protocol-level load, enabling fair apples-to-apples performance comparisons. It serves as the primary instrument for proving Arcane's scaling story.

## Origin & Evolution
The swarm emerged from a need for credible benchmarking methodology during the session focused on proving Arcane's distributed architecture outperforms SpacetimeDB-only at scale. Earlier approaches used HTTP REST polling to simulate clients, which was recognized as fundamentally unfair — it penalized SpacetimeDB by bypassing its native subscription/replication mechanisms entirely. The decision was made to replace that approach with a binary that speaks the real SpacetimeDB protocol (WebSocket + BSATN), mirroring what a genuine game client would do. This reframing was essential to making benchmark results defensible to external audiences, particularly studios evaluating Arcane as infrastructure.

## Technical Details
The swarm binary is built within the `arcane-scaling-benchmarks` workspace and runs as a headless process that spawns a configurable number of simulated clients. Each simulated client connects via WebSocket using the SpacetimeDB SDK, submits subscriptions, and drives the server through reducers at a fixed tick cadence. Canonical workload parameters were locked to ensure reproducibility:

- **Tick rate:** 10 Hz
- **Action rate:** 2 actions/sec per client
- **Run duration:** 30 seconds
- **Movement pattern:** spread movement (not clustered)
- **Visibility model:** everyone-sees-everyone

Key finding: SpacetimeDB-only backend hits a ceiling of approximately 1,000 concurrent players when server-side physics is driven via a `physics_tick` scheduled reducer. The swarm is used to measure where that ceiling manifests and where Arcane's cluster architecture extends it.

## Key Design Decisions
- **Real SDK over HTTP polling** — Using the actual SpacetimeDB WebSocket + BSATN + subscription stack ensures the benchmark reflects true protocol overhead and server behavior, not an artificial REST approximation
- **Canonical workload parameters locked early** — Fixing tick rate, action rate, duration, and visibility model before running experiments prevents parameter drift that would make results incomparable across runs or configurations
- **Headless Rust binary** — Keeps the simulator lightweight and deployable to cloud environments (AWS, Docker) without a game engine dependency, enabling large-scale concurrent simulation
- **Spread movement pattern** — Chosen to stress the spatial indexing and replication fanout rather than create artificial locality that would undercount server load

## Relationships
- [[arcane-infra]]
- [[arcane-core]]
- [[arcane-spatial]]
- [[arcane-rules]]
- [[arcane-pool]]
- [[spacetimedb-comparison]]
- [[arcane-cluster]]
- [[arcane-manager]]

## Conversations That Shaped This
- [[Standalone binary for Unreal Engine testing]]