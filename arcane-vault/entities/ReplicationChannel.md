---
type: entity
tags: [replication, architecture, interface, websocket, state-sync, broadcast, arcane-core, arcane-infra]
---

# ReplicationChannel

## What It Is
`ReplicationChannel` (surfaced as `IReplicationChannel` in the original C++ design, later realized in Rust as a trait and concrete infra implementation) is the interface responsible for propagating entity state updates from ClusterServers to connected clients. In Arcane, it is the boundary between the simulation tick loop and the transport layer — it receives `EntityStateDelta` payloads and delivers them over WebSocket connections to every subscribed client.

## Origin & Evolution
The interface originated in the very first architecture session ([[Unreal Engine setup for networking library]], 2026-02-24), where the four-interface backbone of the library was sketched out: `IClusteringModel`, `IServerPool`, `IReplicationChannel`, and `IWorldSimulator`. At that point the library was still conceptually C++-adjacent; the goal was to decouple Unreal from the authoritative backend so that Unreal became one of several possible client targets rather than the host of game logic.

As the project moved to Rust and the architecture hardened ([[Network library architecture review]], 2026-03-02), `ReplicationChannel` was refined to sit inside `arcane-infra`, with game logic ownership assigned to SpacetimeDB reducers and high-frequency simulation assigned to ClusterServers. This eliminated the need for TCP RPC between clusters for game actions and made `ReplicationChannel` a pure outbound state-push mechanism.

The detailed internal design was investigated in [[STATE_UPDATE message handling in ClusterServer]] (2026-03-16), which traced the full data path and confirmed the **broadcast-first, serialize-once** pattern as the current implementation.

## Technical Details
The replication pipeline inside `arcane-infra` works as follows:

1. **Delta construction** — Each simulation tick, `ClusterServer` builds an `EntityStateDelta` from its local entity map plus pending removals.
2. **Neighbor merge** — `cluster_runner` merges the local delta with neighbor cluster data.
3. **mpsc hand-off** — The merged delta is pushed once over a tokio `mpsc` channel to the WebSocket server component.
4. **Serialize-once** — The WebSocket server serializes the delta to JSON exactly once.
5. **Broadcast fan-out** — The serialized byte string is dropped into a tokio `broadcast` channel; every connected client task subscribes independently and forwards the identical payload.

No per-client filtering, re-serialization, or visibility culling occurs in the hot path. The `in_06_replication_channel_` documentation file (referenced in the 2026-03-16 session) captures the channel contract and backpressure notes; the companion doc `docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md` records validation of the mpsc/broadcast backpressure behavior under load.

The trait lives in **arcane-core** (no I/O); the concrete implementation (WebSocket transport, broadcast channel wiring) lives in **arcane-infra**.

## Key Design Decisions
- **Serialize-once + broadcast** — Rationale: avoids O(n) serialization cost per client; the tradeoff is that all clients receive identical payloads with no per-client culling, which is a known scalability ceiling for large player counts.
- **mpsc between tick loop and WS server** — Rationale: decouples the simulation cadence from the transport layer; backpressure is validated in `WS_CHANNEL_BACKPRESSURE_VALIDATION.md`.
- **Interface defined in arcane-core, implementation in arcane-infra** — Rationale: keeps the trait free of I/O so it can be implemented in tests or alternative transports without pulling in the full infra stack.
- **No per-client visibility filtering in hot path** — Rationale: simplicity and throughput in the initial design; noted explicitly as a future insertion point for spatial culling once the broadcast model is well-understood.
- **Unreal is a client, not the host** — Rationale: the original design pivot (2026-02-24) that made `ReplicationChannel` necessary as an explicit boundary; Unreal consumes state over WebSocket rather than owning replication itself.

## Relationships
- [[ClusterServer]] — produces `EntityStateDelta` each tick and hands it to the channel
- [[EntityStateDelta]] — the payload type the channel carries
- [[IWorldSimulator]] — peer interface in the four-interface backbone
- [[IClusteringModel]] — peer interface
- [[IServerPool]] — peer interface; see [[LocalPool]]
- [[arcane-core]] — defines the `ReplicationChannel` trait
- [[arcane-infra]] — hosts the concrete WebSocket + broadcast implementation
- [[SpacetimeDB]] — authoritative state store; `ReplicationChannel` handles high-frequency simulation state only, not persistent game state
- [[ClusterManager]] — orchestrates ClusterServers whose output feeds the channel

## Conversations That Shaped This
- [[Unreal Engine setup for networking library]] — coined the `IReplicationChannel` interface as part of the four-interface design
- [[Network library architecture review]] — resolved game-logic ownership (SpacetimeDB) and clarified the channel's scope to pure state push
- [[STATE_UPDATE message handling in ClusterServer]] — traced the full internal data path and documented the broadcast-first, serialize-once pattern