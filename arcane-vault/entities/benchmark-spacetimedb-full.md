---
type: entity
tags: [benchmark, spacetimedb, comparison, performance, infrastructure, harness, aws, docker]
---

# benchmark-spacetimedb-full

## What It Is
`benchmark-spacetimedb-full` is one of the benchmark crates in the `arcane-scaling-benchmarks` project, designed to measure SpacetimeDB's standalone performance ceiling without Arcane's cluster layer. It serves as the reference baseline in the Arcane vs. SpacetimeDB comparison, establishing how many concurrent players SpacetimeDB can handle when doing real physics simulation — the honest upper bound for SpacetimeDB-only deployments.

## Origin & Evolution
This crate was created during a major benchmark redesign in the `pgp-demo` session (2026-04-16) after a fundamental flaw was discovered in the original benchmark architecture: SpacetimeDB-only mode was running real physics simulation while the Arcane mode was running none, making comparisons meaningless. To fix this, a five-connection-type architecture was defined and three new benchmark crates — including `benchmark-spacetimedb-full` — were created to produce equivalent, apples-to-apples workloads across both modes. The goal was to establish a true SpacetimeDB ceiling at equivalent simulation fidelity to the Arcane benchmarks.

## Technical Details
The crate runs a full physics simulation workload against a standalone SpacetimeDB instance, mirroring the simulation complexity used in the Arcane benchmark crates. The design ensures parity in workload type so that throughput and player-count ceilings are directly comparable. First production runs established the SpacetimeDB-only ceiling at approximately 250–500 concurrent players, contrasted against Arcane's ceiling of ~2000 players. The benchmark pipeline uses pre-built Docker images pulled from GHCR (migrated away from on-EC2 compilation to reduce cost and complexity) and runs on AWS infrastructure.

## Key Design Decisions
- **Equivalent workload requirement** — the crate must run real physics to match the Arcane benchmark crates; the original flaw of skipping physics in SpacetimeDB mode invalidated earlier comparisons and drove this crate's creation
- **Docker-based execution** — pre-built images from GHCR rather than compiling on EC2, reducing benchmark runtime cost and operational complexity
- **Standalone SpacetimeDB target** — measures SpacetimeDB without any Arcane cluster layer, intentionally establishing the honest ceiling for SpacetimeDB-only deployments rather than a hybrid configuration

## Relationships
- [[arcane-scaling-benchmarks]] — the parent repository containing this crate
- [[benchmark-arcane-full]] — the counterpart crate measuring Arcane's player ceiling
- [[spacetimedb]] — the system under test
- [[arcane-infra]] — the Arcane cluster layer whose performance is compared against this baseline
- [[benchmark-harness]] — the shared harness infrastructure driving all benchmark crates

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]]