---
type: entity
tags: [physics, simulation, rust, dependencies, game-engine, collision-detection, rigid-body]
---

# Rapier

## What It Is
Rapier is a Rust-native physics simulation library used within the Arcane backend for rigid-body dynamics and collision detection. It serves as the physics engine underpinning Arcane's server-side simulation, enabling real physics and combat-grade interactions that are computed authoritatively on the cluster rather than delegated to client-side game engines.

## Origin & Evolution
Rapier was chosen as part of Arcane's core design philosophy: enabling real physics simulation at scale without tying the backend to a specific game engine (Unreal, Unity, Godot). Because Arcane is written in Rust and targets a distributed cluster architecture, a Rust-native physics library was a natural fit. Rapier emerged as the production choice to power the `physics_tick` style simulation workloads that define Arcane's competitive advantage over WASM-based backends like SpacetimeDB, which struggle to scale physics-heavy simulations beyond ~1000 concurrent players.

## Technical Details
Rapier integrates into the Arcane server-side simulation layer, providing rigid-body physics and collision primitives that run within Arcane's tick loop. Because Arcane's architecture is headless and engine-agnostic, Rapier fills the role that a game engine's physics module (e.g., PhysX in Unreal) would play on the client side — but authoritatively on the server. It operates inside the cluster nodes managed by `arcane-infra`, feeding into the replication pipeline that broadcasts state to connected clients (including the Unreal Engine plugin via WebSocket).

## Key Design Decisions
- **Rust-native library** — eliminates FFI overhead and aligns with Arcane's all-Rust backend; no bridging to C++ physics engines required
- **Server-side authority** — physics runs on cluster nodes, not clients, ensuring cheat-resistant, deterministic simulation regardless of client engine
- **Engine-agnostic positioning** — by using Rapier rather than Unreal's PhysX or Unity's physics, Arcane stays compatible with Unity, Unreal, Godot, and custom engine clients simultaneously
- **Scalability enabler** — Rapier's performance in a distributed Rust context is central to Arcane's claim of exceeding the ~1000-player ceiling of single-process backends like SpacetimeDB

## Relationships
- [[arcane-infra]] — crate where cluster simulation and physics tick integration live
- [[arcane-core]] — shared traits and types that physics state would conform to
- [[ClusterServer]] — the node that runs the physics simulation loop
- [[ClusterManager]] — orchestrates which cluster nodes are simulating which regions/entities
- [[SpacetimeDB]] — primary competitor whose physics scaling ceiling Rapier-powered Arcane aims to exceed
- [[arcane-swarm]] — benchmarking binary that stress-tests the physics simulation at scale

## Conversations That Shaped This
- [[Standalone binary for Unreal Engine testing]]