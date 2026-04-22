---
type: entity
tags: [physics, backend, abstraction, architecture, simulation, arcane-core, design-pattern]
---

# Physics Backend Abstraction

## What It Is
The Physics Backend Abstraction is an architectural boundary within Arcane that separates authoritative physics simulation from the core cluster and replication machinery. It defines how physics computation — collision, movement, projectile resolution, and other real-time simulation concerns — plugs into the cluster server infrastructure without being tightly coupled to a single physics engine or implementation. This abstraction is central to Arcane's positioning as an engine-agnostic backend capable of running real physics at scale.

## Origin & Evolution
The abstraction emerged from a foundational problem identified during early architecture sessions: existing multiplayer backends force a binary choice between real physics (dedicated game-engine servers, single-process, player count capped) and horizontal scale (WASM-based services, no real physics). Arcane was designed to break this tradeoff by hosting authoritative physics server-side, distributed across cluster nodes, with the physics layer cleanly separated from replication and state management.

A concrete gap was surfaced during the 2026-03-03 session: the existing benchmarks lacked authoritative physics calculation and only demonstrated toy/placeholder physics. This was called out explicitly as a deficiency that needed to be addressed to validate Arcane's core value proposition. The four-bucket data model developed in that session (Spine, Replicated, Ephemeral, Persistent) also directly shapes how physics state is classified — simulation outputs route through Arcane's replication path, not SpacetimeDB.

## Technical Details
The abstraction sits between the cluster server's tick/simulation loop and any concrete physics engine. Physics state produced by simulation is classified as **Ephemeral** (transient, not persisted) or **Replicated** (broadcast to clients and peer nodes) under the four-bucket model. The mental model is explicit: simulation concerns route to Arcane, persistence concerns route to SpacetimeDB.

The abstraction must support:
- Authoritative state generation per tick (position, velocity, collision results)
- Output that feeds into the replication pipeline toward clients (Unreal, Unity, Godot, custom)
- Engine-agnostic interfaces so studios are not locked to a single physics library

The `arcane-core` crate is the natural home for the trait definitions that form this boundary, since it contains shared types and traits with no I/O. Concrete physics implementations would live in a separate crate or be provided by the integrating studio.

## Key Design Decisions
- **Engine-agnostic interface** — Arcane targets studios using Unreal, Unity, Godot, or custom engines; the physics backend must not assume any particular client or server physics library
- **Authoritative server-side simulation** — physics is computed on cluster nodes, not delegated to clients, which is the core differentiator from WASM-based backend-as-a-service products
- **Four-bucket classification for physics state** — Ephemeral for transient simulation data, Replicated for state that must reach clients; this avoids per-property replication flags and keeps wire overhead low
- **Simulation concerns route to Arcane, persistence to SpacetimeDB** — this boundary prevents physics state from polluting the persistence layer and keeps latency on the hot path low
- **Benchmarks must include real physics** — placeholder/toy physics in benchmarks were explicitly rejected as insufficient to validate the architecture's value proposition

## Relationships
- [[arcane-core]] — trait definitions and shared types that would house the physics backend interface
- [[arcane-infra]] — ClusterServer tick loop that drives physics simulation
- [[Four-Bucket Data Model]] — classifies physics output into Spine, Replicated, Ephemeral, Persistent
- [[Replication Pipeline]] — consumes physics state and broadcasts to clients
- [[SpacetimeDB Integration]] — persistence boundary; physics state explicitly does not route here
- [[arcane-spatial]] — SpatialIndex used for neighbor discovery, feeds into physics proximity queries
- [[ClusterServer]] — the runtime context in which physics ticks execute

## Conversations That Shaped This
- [[Untitled Chat 2026-03-03]] — four-bucket model formalized; toy physics benchmark gap identified; simulation vs. persistence routing boundary established