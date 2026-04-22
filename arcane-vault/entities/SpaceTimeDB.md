---
type: entity
tags: [spacetimedb, architecture, persistence, game-logic, benchmarking, replication, comparison, integration]
---

# SpaceTimeDB

## What It Is
SpacetimeDB is a WASM-based backend-as-a-service platform that Arcane uses as its **authoritative persistence and game-logic layer**. In the Arcane architecture, SpacetimeDB handles discrete game actions and durable state (via reducers), while Arcane's ClusterServers own high-frequency simulation — movement, physics, and AI ticks. SpacetimeDB also serves as the primary **competitive benchmark target**: a key goal of the Arcane project is to demonstrate that its distributed cluster architecture outperforms a SpacetimeDB-only backend at scale.

## Origin & Evolution
SpacetimeDB entered the Arcane story as both a design influence and a foil. The `WHY_ARCANE.md` document explicitly positions Arcane against "WASM-based backend-as-a-service" platforms like SpacetimeDB, identifying their ceiling — roughly **~1,000 concurrent players** in server-side physics benchmarks — as the core problem Arcane solves. Early architecture discussions considered placing game logic in ClusterServers, but the team resolved this tension decisively: **game logic lives in SpacetimeDB reducers**, not in ClusterServers. This eliminated the need for TCP RPC between clusters for game actions and simplified the overall design. Over subsequent sessions, SpacetimeDB evolved from a conceptual comparison point into a live integration target: the swarm benchmarking binary (`arcane-swarm`) was built to use the **actual SpacetimeDB SDK** (WebSocket + BSATN + subscriptions) rather than REST polling, ensuring fair apples-to-apples comparisons.

## Technical Details
- **Protocol:** WebSocket with BSATN binary encoding; clients use subscription-based state delivery.
- **Role in Arcane:** Single authoritative source for persistent game state and discrete game actions. ClusterServers write high-frequency simulation state to Redis; SpacetimeDB handles everything that needs durability or transactional semantics.
- **Reducer pattern:** Scheduled reducers (e.g., `physics_tick`) allow SpacetimeDB to run server-side physics, but this is the configuration benchmarked as the *baseline*, not the Arcane approach.
- **Four-bucket model alignment:** In the canonical data classification — Spine, Replicated, Ephemeral, Persistent — SpacetimeDB owns the **Persistent** bucket. The mental model is: simulation concerns route to Arcane, persistence concerns route to SpacetimeDB.
- **Benchmark ceiling:** SpacetimeDB-only deployments cap at approximately 1,000 concurrent players under the canonical workload (10 Hz tick rate, 2 actions/sec, 30-second runs, everyone-sees-everyone visibility).
- **Cargo feature:** A `spacetimedb-persist` feature flag exists in the workspace (an obsolete version caused a CI breakage that was patched).

## Key Design Decisions
- **Game logic in SpacetimeDB reducers, not ClusterServers** — eliminated TCP RPC between clusters for game actions; ClusterServers stay focused on simulation throughput.
- **SpacetimeDB as persistence owner, not simulation owner** — keeps the hot path (movement, physics) in Rust ClusterServers while retaining SpacetimeDB's transactional guarantees for state that matters across sessions.
- **Swarm client uses real SpacetimeDB SDK** — replaced early HTTP REST polling benchmarks that unfairly penalized SpacetimeDB; ensures benchmark results are credible and comparable.
- **~1,000-player ceiling treated as a hard architectural boundary** — this number anchors Arcane's positioning and drove the distributed cluster design rather than optimizing around SpacetimeDB limits.

## Relationships
- [[ClusterServer]] — owns simulation state; delegates persistence and game actions to SpacetimeDB
- [[ClusterManager]] — orchestrates ClusterServers; sits above the SpacetimeDB integration layer
- [[Redis]] — parallel persistence layer for ephemeral/replicated state; counterpart to SpacetimeDB for transient data
- [[arcane-swarm]] — benchmarking binary that acts as a real SpacetimeDB client to measure scale ceilings
- [[Four-Bucket Data Model]] — canonical classification where SpacetimeDB owns the Persistent bucket
- [[WHY_ARCANE]] — positioning document that frames Arcane vs. SpacetimeDB as the central competitive narrative

## Conversations That Shaped This
- [[Network library architecture review]] — resolved the core tension: game logic in SpacetimeDB reducers, not ClusterServers
- [[Standalone binary for Unreal Engine testing]] — established fair benchmarking methodology; discovered ~1,000-player ceiling; built `arcane-swarm` with real SDK
- [[Untitled Chat]] — formalized four-bucket model; clarified SpacetimeDB as the Persistent bucket owner
- [[CI pipeline failure in Arcane Scaling Benchmarks]] — patched obsolete `spacetimedb-persist` cargo feature that broke CI
- [[Benchmark improvement suggestions]] — deepened the data model and physics architecture in relation to SpacetimeDB's role