---
type: entity
tags: [interface, replication, architecture, rust, core, abstraction, websocket, state-updates]
---

# IReplicationChannel

## What It Is
`IReplicationChannel` is one of four foundational interfaces in Arcane's core design, sitting alongside `IClusteringModel`, `IServerPool`, and `IWorldSimulator`. It defines the contract for how entity state updates flow from simulation (ClusterServers) to connected clients, abstracting the transport layer so that different replication backends (WebSocket, future alternatives) can be swapped without touching game logic.

## Origin & Evolution
The interface emerged during the February 2026 session that established Arcane's standalone architecture. A key early correction shaped it: Arcane was not to be an Unreal plugin but a standalone backend library where Unreal is merely one possible client target. This forced the replication contract to be client-agnostic from the start. The interface design was then stress-tested during the March 2026 architecture review, where the decision to place game logic in SpacetimeDB reducers (not ClusterServers) clarified that `IReplicationChannel` only needs to carry high-frequency simulation state (movement, physics), not discrete game actions. The concrete WebSocket implementation became the reference implementation under `arcane-infra`.

## Technical Details
The current production implementation behind `IReplicationChannel` is a **broadcast-first, serialize-once** pipeline:

1. Each tick, `ClusterServer` constructs an `EntityStateDelta` from local entity state and pending removals.
2. The delta is merged with neighbor cluster data in `cluster_runner`.
3. The merged delta is pushed once over an `mpsc` channel to the WebSocket server component.
4. The WebSocket server serializes the delta to JSON **exactly once** and drops the resulting string into a `tokio::broadcast` channel.
5. Every connected client task subscribes to that broadcast channel and forwards the identical byte payload — no per-client filtering, re-serialization, or visibility culling occurs in the hot path.

Documentation for this subsystem lives in `docs/in_06_replication_channel_*` and `docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md`, which covers backpressure behavior for the mpsc→broadcast handoff.

## Key Design Decisions
- **Serialize-once broadcast** — Avoids N serializations for N clients; accepted tradeoff is that all clients receive the same full-world payload (no per-client culling today).
- **Client-agnostic interface** — Designed from day one to not assume Unreal as the client; Unreal is a consumer of the WebSocket implementation, not a dependency of the interface.
- **High-frequency state only** — Discrete game actions route through SpacetimeDB reducers, keeping `IReplicationChannel` narrowly scoped to tick-rate positional/physics state.
- **No per-client filtering in hot path** — A known scalability limitation; the architecture identifies this mpsc→broadcast boundary as the insertion point for future visibility culling or delta filtering without restructuring the interface.
- **Backpressure documented explicitly** — `WS_CHANNEL_BACKPRESSURE_VALIDATION.md` exists specifically because the broadcast channel drop behavior under slow consumers needed to be validated and understood before production use.

## Relationships
- [[IClusteringModel]]
- [[IServerPool]]
- [[IWorldSimulator]]
- [[ClusterServer]]
- [[EntityStateDelta]]
- [[arcane-infra]]
- [[arcane-core]]
- [[SpacetimeDB]]

## Conversations That Shaped This
- [[Unreal Engine setup for networking library]]
- [[Network library architecture review]]
- [[STATE_UPDATE message handling in ClusterServer]]