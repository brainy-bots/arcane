---
type: entity
tags: [simulation, trait, architecture, core, physics, benchmarks, rust]
---

# ClusterSimulation

## What It Is
`ClusterSimulation` is a Rust trait defined in `arcane-core` that standardizes how game simulation logic integrates with the Arcane cluster backend. It acts as the contract between the infrastructure layer (cluster management, replication, WebSocket handling) and user-supplied game logic, allowing studios to plug in their own physics, combat, or world-simulation code without coupling it to Arcane's internals.

## Origin & Evolution
The trait emerged from a critical flaw discovered during early benchmark work: the SpacetimeDB-only benchmark mode was running real physics while the Arcane mode ran none, making comparisons meaningless. To fix this, a proper simulation interface was needed so equivalent workloads could be driven through both systems. Defining `ClusterSimulation` as an explicit trait was part of the four-PR merge that culminated in publishing `v0.1.0` of the library. It was introduced alongside the entity state model and input validation work as one of the foundational library hardening steps.

## Technical Details
`ClusterSimulation` lives in `arcane-core`, the no-I/O shared-types crate, keeping it free of infrastructure dependencies. Implementors supply the game-specific tick/update logic; the cluster infrastructure calls into them on each simulation step. This separation means the same `ClusterSimulation` implementation can be exercised in a local benchmark harness, a single-process reference server, or a full multi-node AWS deployment without modification. The trait design was part of a broader five-connection-type architecture defined to produce equivalent workloads across Arcane and SpacetimeDB benchmark modes.

## Key Design Decisions
- **Defined in `arcane-core` (no I/O)** — keeps simulation logic portable and testable without pulling in networking or storage dependencies
- **Trait-based contract** — studios implement the trait for their game; Arcane infrastructure calls it, enforcing a clean inversion of control
- **Introduced at v0.1.0** — stabilized early alongside entity state model and input validation so the public API surface was coherent from the first release
- **Benchmark parity requirement** — the trait's existence was directly motivated by needing identical simulation load in Arcane and SpacetimeDB comparison runs; without it, benchmark results were invalid

## Relationships
- [[arcane-core]] — crate where the trait is defined
- [[arcane-infra]] — infrastructure layer that calls into `ClusterSimulation` implementations
- [[ClusterManager]] — orchestrates servers that run simulation instances
- [[ClusterServer]] — hosts and drives the per-instance simulation tick
- [[EntityStateModel]] — the data model simulation logic reads and writes
- [[RulesEngine]] — clustering decisions that may depend on simulation state
- [[LocalPool]] — server pool that may host simulation instances in single-node deployments

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]] — identified the benchmark parity flaw, drove creation of the trait, part of the four-PR v0.1.0 merge
- [[Benchmark improvement suggestions]] — parallel track that clarified Arcane's data model and physics architecture, contextualizing what `ClusterSimulation` needs to support