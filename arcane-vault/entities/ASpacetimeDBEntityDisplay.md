---
type: entity
tags: [unreal-engine, spacetimedb, replication, entity-display, animation, visualization, benchmarking, demo]
---

# ASpacetimeDBEntityDisplay

## What It Is
`ASpacetimeDBEntityDisplay` is an Unreal Engine Actor class in the **arcane-client-unreal** plugin responsible for visualizing replicated game entities that originate from SpacetimeDB. It acts as the client-side representation of a networked entity — receiving position, state, and animation data pushed from the backend and rendering it as a visible, animated character or object in the Unreal scene.

## Origin & Evolution
The class emerged during the 2026-03-06 session focused on building a production-quality Unreal Engine demo to showcase Arcane's distributed cluster architecture handling 150–200+ concurrent networked characters. The core problem was: once the Rust backend was proven to outperform SpacetimeDB-only approaches at scale, the team needed a credible visual demonstration where players could *see* that scale — many entities moving, animating, and being replicated in real time. `ASpacetimeDBEntityDisplay` was designed to serve as the canonical client-side actor spawned and driven by SpacetimeDB subscription callbacks, translating raw BSATN-encoded replication packets into rendered characters with correct animations and positions.

## Technical Details
The actor lives in the arcane-client-unreal Unreal Engine plugin and integrates with the SpacetimeDB Unreal SDK's subscription system. On the backend, entities are ticked at 10 Hz via scheduled reducers and position/state updates are broadcast over WebSocket connections using BSATN encoding. `ASpacetimeDBEntityDisplay` subscribes to the relevant SpacetimeDB table callbacks (row-inserted, row-updated, row-deleted) and uses those events to:
- Spawn or despawn itself when entities enter or leave relevance
- Apply position and rotation updates received from the replication stream
- Drive animation state machines based on replicated action/movement data (e.g., idle vs. moving derived from the 2 actions/sec workload)

The actor is designed to handle the everyone-sees-everyone visibility model used in the benchmarking canonical workload, meaning it must gracefully manage up to ~1000 simultaneous peer entity representations before Arcane's cluster sharding becomes necessary to push beyond that ceiling.

## Key Design Decisions
- **Subscription-driven lifecycle** — actor spawning and despawning is tied directly to SpacetimeDB row-inserted/deleted callbacks rather than a separate polling loop, keeping client state in sync with server state without extra round-trips
- **BSATN-native data path** — the display actor consumes data in the same BSATN format used by the swarm benchmarking clients, ensuring the visual demo exercises the same code path that was benchmarked rather than a special-cased REST or HTTP polling path
- **10 Hz update cadence** — position updates are applied at the backend's canonical tick rate; no client-side extrapolation was initially required for the demo, keeping the first implementation simple
- **Animation driven by replicated state** — rather than inferring animation from positional deltas on the client, action state is replicated explicitly, making animation correct even under packet loss or low update rates

## Relationships
- [[SpacetimeDB]] — the backend source of all entity state this actor displays
- [[arcane-client-unreal]] — the Unreal plugin this actor belongs to
- [[arcane-swarm]] — the headless Rust swarm binary that simulates clients at scale, validating the same replication path this actor visualizes
- [[ClusterManager]] — at high player counts, routes entity data through Arcane's cluster sharding before it reaches SpacetimeDB and then this actor
- [[ArcaneReplicationPipeline]] — the broader data path from Rust backend through SpacetimeDB to this display actor

## Conversations That Shaped This
- [[Standalone binary for Unreal Engine testing]] — the primary session where this actor's role was defined, the benchmarking methodology was locked, and the visual demo requirements were established