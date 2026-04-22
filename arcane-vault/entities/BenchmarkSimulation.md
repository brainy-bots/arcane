---
type: entity
tags: [benchmarks, simulation, rust, architecture, testing, spacetimedb, aws, infrastructure]
---

# BenchmarkSimulation

## What It Is
`BenchmarkSimulation` is the shared simulation workload abstraction used across Arcane's benchmark harness to ensure fair, apples-to-apples comparisons between Arcane and SpacetimeDB backends. It defines what "equivalent work" means in a benchmark run — specifically, that both systems execute the same physics and combat-grade simulation logic rather than one running real simulation while the other runs none.

## Origin & Evolution
The need for `BenchmarkSimulation` emerged from a critical flaw discovered during early benchmark development: the SpacetimeDB-only benchmark mode was running real physics while the Arcane mode was not, making all comparisons meaningless. This was identified and corrected in the 2026-04-16 session (`pgp-demo`), which defined a five-connection-type architecture and created three new benchmark crates to produce equivalent workloads across both backends. The correction was treated as a prerequisite before any benchmark data could be considered valid, and it directly preceded the first production runs that established SpacetimeDB's ceiling at ~250–500 players and Arcane's at ~2000+.

## Technical Details
The simulation abstraction lives inside the `arcane-scaling-benchmarks` workspace and is consumed by multiple benchmark crates. It is designed so that when a benchmark target is exercised — whether Arcane or SpacetimeDB — the same simulation trait implementation is invoked, producing the same computational load. The design ties into the `Simulation` trait added to `arcane-core` as part of the v0.1.0 library hardening (entity state model, simulation trait, input validation). Benchmark runs are containerized via Docker images pulled from GHCR and executed on AWS EC2, replacing on-instance compilation to reduce cost and run-time variance.

## Key Design Decisions
- **Equivalent workload enforcement** — Both Arcane and SpacetimeDB modes must invoke the same physics/simulation logic; this was the core correction that made benchmarks valid.
- **Five-connection-type architecture** — Multiple connection archetypes (distinct by load profile) were defined to stress different parts of each backend under realistic conditions.
- **Three dedicated benchmark crates** — Separating concerns across crates allows each backend target to be measured independently while sharing the simulation core.
- **Docker-based deployment over on-EC2 compilation** — Pre-built images from GHCR reduce AWS cost and eliminate compilation-time variance from benchmark wall-clock measurements.
- **Simulation trait in arcane-core** — Placing the trait in the no-I/O core crate ensures the benchmark simulation implementation has no hidden I/O side effects that could skew results.

## Relationships
- [[arcane-core]] — defines the `Simulation` trait that `BenchmarkSimulation` implements
- [[arcane-scaling-benchmarks]] — the workspace containing the benchmark crates
- [[SpacetimeDB]] — the primary comparison target; early runs established its ~250–500 player ceiling
- [[ClusterManager]] — the Arcane backend component under test, showing ~2000+ player ceiling
- [[RulesEngine]] — clustering decisions exercised during benchmark runs
- [[Docker]] / [[GHCR]] — deployment mechanism for benchmark runners on AWS EC2

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]] — identified the workload equivalence flaw, defined the five-connection-type architecture, created the benchmark crates, executed first valid production runs, and established the initial performance ceilings for both systems