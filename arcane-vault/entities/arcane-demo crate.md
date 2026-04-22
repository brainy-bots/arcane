---
type: entity
tags: [arcane, rust, crate, demo, game-logic, separation-of-concerns, workspace, clustering, replication]
---

# arcane-demo crate

## What It Is
`arcane-demo` is a dedicated Rust crate within the Arcane workspace that houses all game-specific demo behavior — gravity, jumping, wandering agents, demo entity logic — keeping such concerns out of `arcane-infra`. It exists to prove that Arcane is a general-purpose multiplayer backend library, not a demo-first project, by making the boundary between infrastructure and application logic explicit and enforced at the crate level.

## Origin & Evolution
The crate emerged from a 2026-03-03 session focused on a comprehensive architectural refactor. Prior to its creation, demo-specific behavior (wandering NPCs, gravity, jump simulation, agent spawning) lived inside `arcane-infra`, which muddied Arcane's identity as a clean library. The session introduced `arcane-demo` as the designated home for all game logic, simultaneously establishing a new `run_cluster_loop<F>` API in `arcane-infra` that accepts an optional per-tick entity supplier — giving demo code a clean hook point without polluting library internals. Two binaries were produced as a result: `arcane-cluster` (pure infrastructure) and `arcane-cluster-demo` (demo behavior layered on top).

## Technical Details
- **Crate role:** application-layer crate; depends on `arcane-infra` and `arcane-core`, does not export library-facing types
- **Contents:** demo agents, gravity and jump logic, wandering behavior, entity spawning for demonstration purposes
- **Integration point:** hooks into `arcane-infra` via the `run_cluster_loop<F>` generic API, supplying per-tick entity updates as a closure/callback
- **Binary split:**
  - `arcane-cluster` — built from `arcane-infra` alone, no demo logic
  - `arcane-cluster-demo` — built with `arcane-demo`, includes full game-behavior simulation
- **Workspace position:** sits alongside `arcane-core`, `arcane-spatial`, `arcane-rules`, `arcane-pool`, and `arcane-infra` but is explicitly not part of the library surface

## Key Design Decisions
- **Separate crate, not a module** — enforces the library/demo boundary at the Cargo dependency graph level, making it impossible for library consumers to accidentally pull in demo logic
- **`run_cluster_loop<F>` hook** — rather than branching inside `arcane-infra` on whether demo mode is enabled, the generic API inverts control and lets `arcane-demo` supply behavior, keeping `arcane-infra` unaware of game specifics
- **Two binaries** — `arcane-cluster` and `arcane-cluster-demo` let operators and evaluators run the pure infrastructure or the full demo independently, supporting both production use-cases and showcase scenarios
- **Credibility signal** — the separation is explicitly motivated by making Arcane believable as a general-purpose library to studios evaluating it, not just a game demo with library aspirations

## Relationships
- [[arcane-infra crate]]
- [[arcane-core crate]]
- [[run_cluster_loop API]]
- [[arcane-cluster binary]]
- [[arcane-demos repository]]
- [[arcane-client-unreal]]

## Conversations That Shaped This
- [[Project documentation overview]]