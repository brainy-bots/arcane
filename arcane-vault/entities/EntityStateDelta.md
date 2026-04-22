---
type: entity
tags: [replication, state-sync, entity, delta, cluster-server, tick-loop, websocket, arcane-infra, arcane-core]
---

# EntityStateDelta

## What It Is
`EntityStateDelta` is the core replication payload in Arcane's tick-driven state synchronization pipeline. Each tick, a `ClusterServer` constructs an `EntityStateDelta` from its local entity map and pending removals, representing the changes to entity state that need to be broadcast to connected clients. It is the fundamental unit of state transfer between the simulation layer and the network layer.

## Origin & Evolution
The concept emerged from a key architectural decision made during the [[Network library architecture review]] (2026-03-02): game logic lives in SpacetimeDB reducers while ClusterServers own high-frequency simulation state (movement, physics, AI ticks). This separation created a clear need for a structured delta type that could carry owned entity state from a ClusterServer's local simulation outward to clients — without conflating persistent game actions with per-tick positional updates. The `EntityStateDelta` became the concrete expression of that boundary. Its data path was fully traced in [[STATE_UPDATE message handling in ClusterServer]] (2026-03-16), where the broadcast-first, serialize-once pattern was documented and its scalability properties were evaluated.

## Technical Details
Each tick, `ClusterServer` constructs an `EntityStateDelta` from two sources: its local entity map (live entity states) and a pending removals list. The delta is then merged with neighbor cluster data inside `cluster_runner`, producing a combined delta that represents the full world view for a given cluster's clients. This merged delta is pushed once over an `mpsc` channel to the WebSocket server, which serializes it to JSON exactly once and drops the resulting string into a `tokio` broadcast channel. Every connected client task subscribes to that broadcast channel and forwards the identical byte payload — no per-client filtering, re-serialization, or visibility culling occurs in the hot path. The flow is:

```
entity map + removals
        ↓
  EntityStateDelta  (per ClusterServer, per tick)
        ↓
  cluster_runner merge (own + neighbor deltas)
        ↓
  mpsc → WS server → serialize-once → tokio broadcast
        ↓
  every client task → forward bytes
```

The design lives in `arcane-infra` but the type itself is defined in `arcane-core` to keep it free of I/O dependencies.

## Key Design Decisions
- **Broadcast-first, serialize-once** — JSON serialization happens once per tick regardless of client count; this eliminates per-client serialization cost but also means no per-client payload differentiation exists today
- **Delta includes removals explicitly** — pending removals are tracked separately from the entity map and included in the delta, ensuring clients can tombstone entities without relying on absence heuristics
- **Neighbor merge before broadcast** — own-cluster delta and neighbor-cluster data are merged in `cluster_runner` before entering the WS pipeline, so clients receive a unified world snapshot rather than per-cluster fragments
- **No per-client filtering in hot path** — visibility culling and interest management are explicitly deferred; the current model is intentionally simple and the merge/broadcast architecture is documented as the insertion point for future filtering

## Relationships
- [[ClusterServer]] — constructs the delta each tick from its local entity map and removals
- [[cluster_runner]] — merges own and neighbor deltas before pushing to the WS layer
- [[SpacetimeDB]] — authoritative source for persistent state; `EntityStateDelta` carries only simulation-layer (high-frequency) state, not discrete game actions
- [[SpatialIndex]] — neighbor discovery that determines which cluster deltas are merged
- [[WebSocket broadcast channel]] — the tokio broadcast channel that distributes the serialized delta to all client tasks
- [[STATE_UPDATE]] — the wire message type that wraps the serialized `EntityStateDelta` payload

## Conversations That Shaped This
- [[Network library architecture review]] — established the ClusterServer/SpacetimeDB ownership split that motivated the delta type
- [[STATE_UPDATE message handling in ClusterServer]] — traced the full data path and documented the broadcast-first, serialize-once pattern and its scalability implications