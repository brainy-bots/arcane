---
type: entity
tags: [benchmark, spacetimedb, persistence, performance, arcane, rust, infrastructure]
---

# benchmark-spacetimedb-persist

## What It Is
`benchmark-spacetimedb-persist` is one of the benchmark crates in the `arcane-scaling-benchmarks` workspace that measures SpacetimeDB's write-through persistence performance as an isolated baseline. It exercises SpacetimeDB's WASM module + database persistence path — committing entity state to durable storage — without Arcane's cluster layer, establishing the ceiling throughput and latency that SpacetimeDB alone can sustain before Arcane's distributed architecture is added on top.

## Origin & Evolution
This crate was created during a fundamental redesign of the benchmark suite discovered in the pgp-demo session (2026-04-16). The original benchmark harness had a critical flaw: SpacetimeDB-only mode ran real physics simulation while Arcane mode ran none, making any comparison meaningless. To fix this, a five-connection-type architecture was defined and multiple new benchmark crates were created to produce equivalent, apples-to-apples workloads across both SpacetimeDB-only and Arcane modes. `benchmark-spacetimedb-persist` specifically isolates the persistence-write path so that its cost can be measured independently and subtracted (or compared) against full-stack numbers.

## Technical Details
- Runs against a live SpacetimeDB instance (local or AWS-hosted Docker container pulled from GHCR)
- Exercises the WASM module path: client messages → SpacetimeDB reducer → committed state writes
- Produces throughput (players sustained) and latency metrics comparable to Arcane's equivalent benchmark
- Part of the broader benchmark pipeline that migrated from on-EC2 compilation to pre-built Docker images to reduce AWS runtime cost
- Results contributed to establishing the SpacetimeDB-only ceiling at approximately 250–500 concurrent players, against which Arcane's ~2000-player ceiling was measured

## Key Design Decisions
- **Isolated persistence path** — benchmarks only the SpacetimeDB write/commit loop, not a full game simulation, so persistence overhead is visible without simulation noise confounding results
- **Equivalent workload contract** — workload shape is defined by the five-connection-type architecture to be structurally identical to the Arcane-side benchmarks, making cross-stack comparison valid
- **Docker-based execution** — SpacetimeDB runs in a container pulled from GHCR rather than compiled on the benchmark host, keeping runtime reproducible and AWS costs low
- **Separation from physics benchmarks** — physics simulation cost is benchmarked in a sibling crate; persistence is a distinct axis so both can be independently profiled

## Relationships
- [[benchmark-spacetimedb-physics]] — sibling crate benchmarking SpacetimeDB with real physics; together they cover the two main SpacetimeDB-only workload axes
- [[benchmark-arcane-cluster]] — the Arcane-side counterpart measuring the full distributed cluster path
- [[arcane-scaling-benchmarks]] — parent workspace containing all benchmark crates and the shared harness
- [[spacetimedb]] — the system under test; WASM-based backend-as-a-service whose persistence model is what this benchmark measures
- [[aws-benchmark-pipeline]] — the AWS infrastructure (EC2 + Docker + GHCR) used to execute production benchmark runs
- [[five-connection-type-architecture]] — the workload design contract that ensures all benchmark crates produce comparable loads

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]] — the session where the benchmark design flaw was identified, the five-connection-type architecture was defined, and this crate (along with its siblings) was created to replace the broken comparison baseline