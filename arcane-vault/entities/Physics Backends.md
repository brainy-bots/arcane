---
type: entity
tags: [physics, architecture, backends, simulation, scaling, design-decisions]
---

# Physics Backends

## What It Is
Physics Backends refers to the pluggable simulation layer within Arcane that decouples the physics/combat engine from the multiplayer backend infrastructure. Rather than tying Arcane to a single physics runtime, the design allows game studios to bring their own physics implementation — or swap between backends — while Arcane handles cluster management, replication, and spatial indexing around it. This is central to Arcane's pitch: delivering real physics simulation at player counts that single-process dedicated game-engine servers cannot reach.

## Origin & Evolution
The need for Physics Backends emerged directly from the core problem Arcane was built to solve. Traditional dedicated game-engine servers (Unreal, Unity) provide real physics but are single-process and therefore hard-capped in player count. WASM-based backends like SpacetimeDB offer scale but sacrifice real simulation. Arcane's answer was to treat the physics layer as a concern separate from the infrastructure layer — the backend scales the cluster, the physics backend does the simulation. The benchmark and architecture sessions (notably the 2026-03-30 session) clarified how this division manifests in practice: physics state flows through the replication and spatial systems rather than being owned by the transport or cluster management layer.

## Technical Details
Arcane's crate structure reflects the separation of concerns around physics. `arcane-spatial` provides the `SpatialIndex` (2D grid) for neighbor discovery, which feeds into clustering decisions made by `arcane-rules` (`RulesEngine`) — both of which are physics-backend-agnostic. The actual simulation state is treated as replicated data managed by `arcane-infra` (`ClusterManager`, `ClusterServer`) with Redis and SpacetimeDB as persistence/replication targets. The physics backend itself is expected to produce and consume this state, but the interfaces in `arcane-core` (traits and shared types, no I/O) are the contract boundary. No single physics runtime is bundled; the architecture assumes an external or user-supplied simulation layer integrates at the state model level.

## Key Design Decisions
- **Decoupled simulation from transport** — Physics state is replicated data, not a function of the WebSocket or cluster transport layer; this lets the physics backend evolve independently of network topology.
- **Engine-agnostic client contract** — Because physics is not embedded in Arcane's core, studios using Unity, Unreal, Godot, or custom engines can all integrate against the same backend without the backend dictating simulation fidelity.
- **Spatial indexing as infrastructure, not physics** — `arcane-spatial`'s `SpatialIndex` is a neighbor-discovery utility for clustering decisions, explicitly not a physics primitive; this keeps the rules engine light and composable.
- **State model owns physics output** — The 2026-03-30 architecture session identified that physics state must be modeled as a first-class replicated entity, not derived on-the-fly, to support consistent replication across cluster nodes.
- **No WASM constraint** — Unlike SpacetimeDB's scripting model, Arcane does not require physics logic to run inside a WASM sandbox, preserving access to native performance-critical simulation libraries.

## Relationships
- [[arcane-core]]
- [[arcane-spatial]]
- [[arcane-rules]]
- [[arcane-infra]]
- [[ClusterManager]]
- [[SpatialIndex]]
- [[RulesEngine]]
- [[Replication]]
- [[State Model]]
- [[SpacetimeDB]]

## Conversations That Shaped This
- [[Benchmark improvement suggestions]]