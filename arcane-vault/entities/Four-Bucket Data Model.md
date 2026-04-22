---
type: entity
tags: [data-model, replication, architecture, cluster-server, spacetimedb, redis, core-design]
---

# Four-Bucket Data Model

## What It Is
The Four-Bucket Data Model is Arcane's canonical framework for classifying all data that flows through cluster servers into four distinct categories: **Spine**, **Replicated**, **Ephemeral**, and **Persistent**. It defines the lifecycle, replication behavior, and storage destination for every piece of state in the system, serving as the conceptual backbone for how developers reason about data when building on Arcane.

## Origin & Evolution
The model emerged from a 2026-03-03 design session focused on how custom code should interact with cluster server state. The conversation began with a more granular approach — Unreal Engine–style per-property replication flags — but the team rejected this in favor of a cleaner, more opinionated design. The core insight was that per-property flags add metadata overhead on the wire and force developers to reason about replication at too fine a granularity. By elevating the classification to the type level, the model makes replication rules explicit and architectural rather than incidental. The session produced the four-bucket taxonomy as the canonical output, with the guiding mental model: **simulation concerns route to Arcane; persistence concerns route to SpacetimeDB**.

## Technical Details
Each bucket defines a distinct contract for how data is handled:

| Bucket | Description | Destination |
|--------|-------------|-------------|
| **Spine** | Authoritative simulation state — position, velocity, physics results. Lives in the cluster server, drives the simulation tick. | Arcane cluster (in-memory, authoritative) |
| **Replicated** | State that must be broadcast to clients each tick or on change — visible game state derived from or alongside Spine. | Arcane replication pipeline (→ clients) |
| **Ephemeral** | Transient per-tick data that does not need to survive beyond the current frame — intermediate calculations, local signals. | Discarded after tick; never persisted or replicated |
| **Persistent** | Durable state that must survive server restarts, session boundaries, or cluster reassignment — inventory, progression, match history. | SpacetimeDB (via write-through) |

The model is enforced at the type level in Rust: data is placed in a bucket by its type declaration, not by per-field annotations. This eliminates a class of replication bugs where individual fields are misconfigured.

## Key Design Decisions
- **Type-level classification over per-property flags** — eliminates wire overhead from per-field replication metadata and makes the replication contract visible at the struct definition, not scattered across fields
- **Simulation/persistence split as the primary axis** — routing Spine and Replicated to Arcane and Persistent to SpacetimeDB reflects a clean separation of concerns: Arcane owns the live simulation, SpacetimeDB owns durable truth
- **Ephemeral as a first-class bucket** — explicitly naming throwaway state prevents accidental persistence or replication of intermediate values, which is a common source of bugs in simulation backends
- **Rejected: Unreal-style per-property replication flags** — considered and discarded because they push replication reasoning to the field level, increase complexity for game developers, and add unnecessary wire overhead

## Relationships
- [[ClusterServer]] — the runtime that hosts and executes data classified by this model
- [[SpacetimeDB Integration]] — the designated destination for Persistent bucket data
- [[Redis Replication Layer]] — participates in propagating Replicated bucket data across the cluster
- [[Replication Pipeline]] — the mechanism that carries Replicated data from cluster servers to clients
- [[arcane-core]] — the crate where bucket traits and shared types are expected to live

## Conversations That Shaped This
- [[Untitled Chat 2026-03-03]] — the founding session; produced the four-bucket taxonomy, rejected per-property flags, and established the simulation/persistence mental model