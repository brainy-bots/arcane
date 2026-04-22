---
type: entity
tags: [rust, crate, clustering, rules-engine, architecture, arcane-core]
---

# arcane-rules

## What It Is
`arcane-rules` is a dedicated Rust crate within the Arcane workspace that houses the `RulesEngine` — the component responsible for making clustering decisions. It determines how entities, players, and simulation workloads should be distributed and grouped across ClusterServers in the backend topology.

## Origin & Evolution
The crate emerged from the need to separate policy (which server should own what, when should clusters split or merge) from mechanism (how servers communicate, how state is replicated). By isolating clustering logic into its own crate with no I/O dependencies, the rules can be tested and evolved independently of the infrastructure layer. During the 2026-03-02 architecture review, the broader clustering design was substantially refined — resolving tensions around state ownership, entity lifecycle, and clustering cadence — which sharpened the requirements for what the RulesEngine needed to decide and when.

## Technical Details
`arcane-rules` is a pure-logic crate: it contains the `RulesEngine` and operates without I/O, keeping it fast to compile and easy to unit-test. It sits above `arcane-core` (which provides the shared traits and types it reasons over) and below `arcane-infra` (which invokes the engine to drive actual cluster management decisions). The `ClusterManager` in `arcane-infra` consumes the `RulesEngine` to evaluate clustering state and decide on assignments, splits, and merges at runtime.

## Key Design Decisions
- **No I/O in the crate** — keeps rules deterministic and fully unit-testable without mocking network or storage layers
- **Separate crate, not a module** — enforces a hard boundary between clustering policy and infrastructure mechanism; changes to rules cannot accidentally couple to transport code
- **Depends only on arcane-core** — ensures shared types flow one way through the dependency graph, preventing circular dependencies
- **RulesEngine as the single decision point** — centralizing clustering decisions in one component makes policy auditable and replaceable without touching replication or server-pool logic

## Relationships
- [[arcane-core]] — provides the traits and shared types that `arcane-rules` reasons over
- [[arcane-infra]] — the consumer; `ClusterManager` calls the `RulesEngine` to make live clustering decisions
- [[arcane-pool]] — `LocalPool` manages the server pool whose state the `RulesEngine` evaluates
- [[arcane-spatial]] — `SpatialIndex` provides neighbor/proximity data that feeds clustering decisions
- [[ClusterManager]] — the runtime owner that invokes `RulesEngine` results to act on the cluster

## Conversations That Shaped This
- [[Network library architecture review]] — resolved the ten major architectural tensions that define what clustering decisions need to be made, directly shaping the RulesEngine's scope
- [[Claude Code session — pgp-demo]] — orientation session that confirmed the five-crate workspace structure and `arcane-rules`' place within it