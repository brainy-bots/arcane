---
type: entity
tags: [replication, channels, rust, arcane-infra, websocket, backpressure, concurrency]
---

# ReplicationChannelManager

## What It Is
`ReplicationChannelManager` is the component within `arcane-infra` responsible for managing the lifecycle and routing of replication channels — the conduits through which entity state updates flow from ClusterServers to connected clients. It coordinates per-entity or per-subscription channels, ensuring that high-frequency simulation state (movement, physics, AI ticks) produced by ClusterServers reaches the right clients efficiently and without overwhelming downstream consumers.

## Origin & Evolution
The manager emerged from the need to decouple high-frequency simulation writes from the WebSocket fan-out layer. As the architecture review (2026-03-02) established that ClusterServers own simulation state and write it at tick rate, a dedicated coordination layer became necessary to buffer, route, and apply backpressure before state hits the WebSocket transport. Without this layer, a burst of simulation ticks could saturate the send buffers of slow or disconnected clients, destabilizing the whole cluster node. The backpressure validation work documented in `docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md` reflects the effort to formally characterize and test these guarantees.

## Technical Details
- Lives in `arcane-infra`, the crate that owns `ClusterManager`, `ClusterServer`, and the replication subsystem.
- Manages channels that carry entity state deltas from simulation tick loops to the WebSocket egress path.
- Backpressure behavior is governed by bounded async channels; when a subscriber's channel is full, the manager applies a defined policy (drop, queue, disconnect) rather than blocking the simulation tick.
- Interacts with `ClusterServer` as the producer side and with the WebSocket connection handler as the consumer side.
- State flowing through these channels originates from ClusterServer-owned entities; persistent/discrete game actions are NOT routed here — those live in SpacetimeDB reducers.
- Architecture and backpressure validation notes are captured in `docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md`.

## Key Design Decisions
- **Bounded channels over unbounded** — prevents slow clients from causing unbounded memory growth on the server; backpressure is explicit and measurable.
- **Simulation logic stays out of the channel layer** — the manager is pure routing/lifecycle; game logic belongs in SpacetimeDB reducers (discrete actions) or ClusterServer tick loops (simulation), not here.
- **Per-subscription channel isolation** — a stalled or disconnected client's channel does not block replication for other subscribers.
- **Observability-first** — channel saturation, drop rates, and consumer lag are instrumented as first-class metrics, consistent with the production posture established in the 2026-03-02 architecture review.

## Relationships
- [[ClusterServer]] — primary producer of entity state that flows through managed channels
- [[ClusterManager]] — orchestrates ClusterServers; aware of replication topology
- [[arcane-infra]] — parent crate housing this component
- [[SpatialIndex]] — neighbor discovery feeds subscription decisions that determine which channels a client receives
- [[WebSocket Transport]] — consumer side of the channels managed here
- [[Redis]] — used for cross-node state propagation; complements but is separate from intra-node channel management

## Conversations That Shaped This
- [[Network library architecture review]] — established the simulation-ownership model, SpacetimeDB/ClusterServer split, and the need for a dedicated replication routing layer with explicit backpressure