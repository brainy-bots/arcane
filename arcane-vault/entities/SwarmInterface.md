---
type: entity
tags: [rust, traits, clustering, swarm, abstraction, arcane-core, infra, distributed-systems]
---

# SwarmInterface

## What It Is
`SwarmInterface` is a core trait (or trait family) in Arcane that defines the contract between the clustering orchestration layer and the underlying server pool or swarm of `ClusterServer` instances. It abstracts over how servers are discovered, spawned, drained, and coordinated — allowing the `ClusterManager` to make clustering decisions without being coupled to any specific pool implementation (local, cloud, Kubernetes, etc.).

## Origin & Evolution
The need for `SwarmInterface` emerged as Arcane's architecture resolved the tension between clustering decision logic (owned by `arcane-rules`) and actual server lifecycle management (owned by `arcane-pool` and `arcane-infra`). Early designs risked tightly coupling `ClusterManager` to `LocalPool`, making alternative deployment backends impossible. By extracting a swarm-level interface, the system became testable in isolation — `arcane-core` could define the trait with no I/O, and concrete implementations could live in `arcane-infra`. The visualization work (February 2026) that introduced server-spawning-on-convergence as a scaling signal made the need for a clean spawn/drain contract explicit: the clustering engine needed to *request* capacity changes without knowing how they were fulfilled.

## Technical Details
- Defined in **`arcane-core`** — the no-I/O traits crate — so it carries no runtime dependencies
- Implemented by concrete pool types in **`arcane-pool`** (`LocalPool`) and potentially `arcane-infra` for distributed deployments
- The interface covers at minimum: listing available servers, requesting a new server be spawned, draining/removing a server, and querying server load or health signals
- `ClusterManager` holds a reference to something implementing `SwarmInterface`, enabling it to act on `RulesEngine` outputs (merge, split, rebalance) by issuing capacity commands
- Designed to support both the reference single-machine deployment (binary `arcane-cluster`) and future cloud-native swarm backends without changing clustering logic

## Key Design Decisions
- **Defined in `arcane-core`, not `arcane-infra`** — keeps the trait free of I/O and makes it mockable for TDD; clustering logic can be unit-tested without spinning up real servers
- **Abstracts over pool implementation** — `LocalPool` satisfies the interface for local/demo use; a Kubernetes or bare-metal pool can satisfy the same interface for production, with zero changes to `RulesEngine` or `ClusterManager`
- **Spawn-on-convergence as a first-class operation** — the February 2026 visualization work established that server spawning is a clustering signal response, not an administrative action; the interface reflects this by making spawn/drain callable from within the clustering loop
- **Load as a query, not a push** — server load is polled through the interface rather than pushed, keeping the data flow unidirectional and avoiding the race conditions that plagued earlier push-based designs

## Relationships
- [[ClusterManager]] — primary consumer; calls `SwarmInterface` methods to enact clustering decisions
- [[RulesEngine]] — produces clustering decisions that `ClusterManager` translates into `SwarmInterface` calls
- [[LocalPool]] — reference implementation of `SwarmInterface` for single-machine and demo deployments
- [[ClusterServer]] — the unit being managed; `SwarmInterface` operates over collections of these
- [[arcane-core]] — crate where the trait is defined
- [[arcane-pool]] — crate providing `LocalPool` implementation
- [[arcane-infra]] — crate wiring `ClusterManager` + `SwarmInterface` into binaries

## Conversations That Shaped This
- [[Network library architecture review]] — resolved the ten major architectural tensions; established that `ClusterManager` must be decoupled from concrete pool types and that game logic lives outside ClusterServers, clarifying exactly what `SwarmInterface` needs to expose
- [[Untitled Chat]] — introduced server-spawning-on-convergence as a behavioral requirement, making spawn/drain first-class operations in the swarm contract rather than afterthoughts