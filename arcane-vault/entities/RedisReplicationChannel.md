---
type: entity
tags: [redis, replication, infrastructure, pub-sub, cluster, arcane-infra, state-sync]
---

# RedisReplicationChannel

## What It Is
`RedisReplicationChannel` is the pub/sub transport layer inside `arcane-infra` that propagates entity state updates between `ClusterServer` nodes via Redis. It allows each server to broadcast owned entity state (positions, physics, AI ticks) and receive updates about entities owned by peer servers, enabling cross-cluster visibility without direct TCP connections between nodes.

## Origin & Evolution
The channel emerged as a solution to a core distributed systems problem: multiple `ClusterServer` instances each own a slice of the simulation, but clients and game logic need a coherent view of entities across all servers. Early design considered TCP RPC between clusters for state sharing, but this was rejected during the 2026-03-02 architecture review as it introduced tight coupling and failover complexity. Redis pub/sub was chosen instead because it decouples producers from consumers, scales horizontally, and fits naturally into the existing infrastructure stack alongside SpacetimeDB. The decision to place game logic in SpacetimeDB reducers (not ClusterServers) further simplified the replication contract — the channel only needs to carry high-frequency simulation state (movement, physics) rather than authoritative game actions.

## Technical Details
`RedisReplicationChannel` lives in `arcane-infra` alongside `ClusterManager` and `ClusterServer`. Each `ClusterServer` publishes state deltas for its owned entities to a Redis channel and subscribes to channels for entities owned by peers. The channel operates as a fire-and-forget broadcast; authoritative persistent state remains in SpacetimeDB, so the replication channel carries ephemeral high-frequency updates only. Backpressure behavior and channel validation are documented in `docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md`, which covers how the WS/channel pipeline handles slow consumers and overflow.

## Key Design Decisions
- **Redis pub/sub over TCP RPC between clusters** — eliminates direct inter-cluster coupling; servers can come and go without needing to manage peer connection state
- **Ephemeral state only** — persistent/authoritative game state lives in SpacetimeDB; the replication channel carries only high-frequency simulation deltas, keeping the channel's failure domain isolated
- **Game logic excluded from ClusterServer** — by confining game logic to SpacetimeDB reducers, the replication channel contract stays narrow (simulation state, not game actions), reducing the blast radius of replication bugs
- **Observability-first** — the architecture review mandated production observability; the channel is expected to expose metrics on publish/subscribe rates, lag, and dropped messages

## Relationships
- [[ClusterServer]] — publishes to and consumes from the channel per owned entity partition
- [[ClusterManager]] — coordinates which server owns which entities, informing channel routing
- [[SpacetimeDB]] — authoritative persistent state store; complements the ephemeral replication channel
- [[arcane-infra]] — the crate that houses this component alongside cluster binaries
- [[LocalPool]] — server pool whose members interact via the replication channel
- [[SpatialIndex]] — neighbor discovery output can inform which entity states need cross-cluster replication

## Conversations That Shaped This
- [[Network library architecture review]]