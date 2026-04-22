---
type: entity
tags: [architecture, benchmarks, wire-protocol, spacetimedb, connection-types, design]
---

# Five Connection Types

## What It Is
The Five Connection Types is an architectural taxonomy that defines five distinct benchmark workload modes used to produce equivalent, apples-to-apples comparisons between Arcane and SpacetimeDB. Each type represents a specific client-connection topology and physics execution context, ensuring that benchmark runs across both systems are measuring the same real workload rather than incompatible configurations.

## Origin & Evolution
The five-connection-type architecture emerged from a critical flaw discovered during the `pgp-demo` session (2026-04-16): early benchmark runs were fundamentally broken because SpacetimeDB-only mode executed real physics simulation while Arcane mode did not, making comparisons meaningless. To fix this, the team defined a structured set of five connection types and created three new benchmark crates to implement equivalent workloads across both backends. This work was part of the broader effort to produce credible, publishable benchmark data for Arcane v0.1.0.

## Technical Details
Three new benchmark crates were built around the five connection types to ensure workload equivalence. The taxonomy standardizes what "a connection" means in each mode — covering variations in whether physics is real or simulated, whether the server is Arcane or SpacetimeDB, and what the client topology looks like. This design directly enabled the first production benchmark runs, which established SpacetimeDB's ceiling at ~250–500 players and Arcane's ceiling at ~2,000 players under comparable loads.

## Key Design Decisions
- **Explicit taxonomy before implementation** — defining types first prevented the benchmark drift that caused the original flawed comparisons; workload equivalence had to be a first-class constraint, not an afterthought.
- **Three separate benchmark crates** — each crate targets a specific subset of the five types, keeping workload logic isolated and independently verifiable rather than entangled in a single harness.
- **Real physics parity** — both Arcane and SpacetimeDB benchmark paths were required to run real physics (not stubs), enforcing that throughput differences reflect architectural differences, not simulation shortcuts.

## Relationships
- [[ClusterManager]]
- [[ClusterServer]]
- [[SpacetimeDB Integration]]
- [[Arcane Scaling Benchmarks]]
- [[Simulation Trait]]
- [[Entity State Model]]

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]]