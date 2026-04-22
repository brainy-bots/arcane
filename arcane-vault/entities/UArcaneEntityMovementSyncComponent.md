---
type: entity
tags: [unreal-engine, replication, movement, networking, component, client-plugin, arcane-client-unreal]
---

# UArcaneEntityMovementSyncComponent

## What It Is
`UArcaneEntityMovementSyncComponent` is an Unreal Engine Actor Component, part of the **arcane-client-unreal** plugin, responsible for synchronizing entity movement state received from the Arcane backend to local Unreal Engine actors. It bridges the gap between the Arcane cluster's authoritative replication stream and Unreal's native movement and animation systems, driving character positions, velocities, and animation states for remotely-owned entities.

## Origin & Evolution
The component emerged from the need to render 150–200+ concurrent networked characters in the Unreal Engine demo showcasing Arcane's distributed cluster architecture. Early approaches to client-side replication were too tightly coupled to raw network messages; the component abstraction was introduced to cleanly own the "receive replicated state → apply to Unreal actor" responsibility. As the session progressed, the component became the canonical consumer of entity snapshots pushed by the Arcane [[ClusterServer]] replication stream, decoupling animation and movement logic from the lower-level WebSocket/message-parsing layer.

## Technical Details
- Lives in the **arcane-client-unreal** Unreal Engine plugin, added to a project via `Plugins/`.
- Attaches to an Actor that represents a remotely-owned entity (i.e., not the locally-controlled player pawn).
- Consumes replicated entity snapshots — position, velocity, facing direction, animation state — delivered by the plugin's connection subsystem.
- Applies received state to Unreal's `UCharacterMovementComponent` or directly to the actor's transform, depending on whether the entity uses a `ACharacter` base or a lighter `AActor`.
- Tick rate on the client mirrors the server's 10 Hz replication cadence; the component interpolates between received snapshots each render frame to produce smooth motion.
- Animation state fields from the snapshot (e.g., locomotion blend space inputs) are forwarded to the owning actor's Animation Blueprint via exposed properties or a delegate, keeping animation logic in Blueprint while keeping network parsing in C++.
- Designed to handle 150–200+ simultaneous component instances without per-frame allocation; snapshot structs are value-copied from a pooled receive buffer.

## Key Design Decisions
- **Component, not subsystem** — Movement sync is per-actor state, so a component is the natural Unreal primitive; a subsystem would require a manual actor-to-state mapping table.
- **Interpolation over extrapolation** — At 10 Hz the jitter window is predictable; the component holds the last two snapshots and lerps, avoiding the drift and correction artefacts of dead-reckoning extrapolation.
- **Animation state passed as data, not calls** — The component does not call animation functions directly; it writes to shared properties read by the Animation Blueprint, preserving Blueprint authorship of animation logic.
- **Separation from local player pawn** — The component is intentionally not placed on the locally-controlled pawn; local movement uses Unreal's standard `UCharacterMovementComponent` with server reconciliation handled elsewhere, keeping authority paths clean.
- **10 Hz tick alignment** — Canonical workload parameters (10 Hz tick, 2 actions/sec) were locked early in benchmarking to ensure the client-side interpolation budget matched the server replication cadence.

## Relationships
- [[arcane-client-unreal]] — the Unreal plugin this component belongs to
- [[ClusterServer]] — Arcane backend component whose replication stream produces the snapshots this component consumes
- [[arcane-infra]] — Rust crate housing `ClusterServer` and the replication subsystem
- [[arcane-swarm]] — headless Rust benchmarking binary that simulates the same entity movement workload this component visualises

## Conversations That Shaped This
- [[Standalone binary for Unreal Engine testing]]