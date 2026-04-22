---
type: entity
tags: [wire-protocol, networking, serialization, arcane-infra, benchmarks, websocket]
---

# arcane-wire

## What It Is
`arcane-wire` is the wire protocol layer for the Arcane multiplayer backend, defining the serialization format and message types used for communication between clients, ClusterManager, and ClusterServers. It provides the structured message envelopes that travel over WebSocket connections, ensuring that the Arcane client plugin and backend speak the same language at the byte level.

## Origin & Evolution
The wire protocol emerged as a formalization need during the benchmark harness work in the `pgp-demo` session (2026-04-16), where the project required equivalent, comparable workloads across Arcane and SpacetimeDB modes — which demanded a well-defined and stable message contract. Before this point, message shapes were implicit in the infra layer; pulling them into an explicit protocol artifact allowed benchmark clients to generate valid Arcane traffic without embedding business logic. The v0.1.0 publication milestone marked when the protocol was considered stable enough for external reference.

## Technical Details
- Sits at the boundary between `arcane-infra` (which owns the WebSocket server and channel machinery) and the client layer (`arcane-client-unreal` and benchmark harness clients)
- Messages travel over WebSocket connections between clients and `ClusterServer`, and between `ClusterManager` (HTTP join) and cluster nodes
- The protocol must be consistent enough for the benchmark harness to replay realistic player workloads, including spatial updates, input events, and state replication payloads
- Backpressure behavior at the WS/channel boundary is documented in `docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md`, indicating the protocol is aware of flow-control constraints
- Serialization format choices (binary vs. text, schema versioning) are owned here and affect both Unreal client integration and benchmark fidelity

## Key Design Decisions
- **Explicit protocol artifact** — pulling message definitions out of infra and into a dedicated layer prevents clients from being coupled to internal server types and allows independent versioning
- **Engine-agnostic framing** — the wire format is not tied to Unreal or any specific client engine, consistent with Arcane's goal of supporting Unity, Unreal, Godot, and custom engines
- **Benchmark-driven validation** — the benchmark harness acts as a second consumer of the protocol, which forced early stability and caught ambiguities that a single-client design would have deferred
- **Backpressure awareness** — the protocol design accounts for WS channel saturation, with documented behavior rather than silent drops, keeping the protocol contract honest under load

## Relationships
- [[arcane-infra]] — hosts the WebSocket server and consumes wire message types for routing and replication
- [[arcane-core]] — shared traits and types that wire message schemas may reference
- [[arcane-client-unreal]] — primary production client; must implement the same wire contract
- [[ClusterServer]] — the endpoint that receives and dispatches wire messages from clients
- [[ClusterManager]] — uses HTTP (join flow) alongside the wire protocol for cluster coordination
- [[arcane-scaling-benchmarks]] — second consumer of the wire protocol; drove protocol stabilization

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]]