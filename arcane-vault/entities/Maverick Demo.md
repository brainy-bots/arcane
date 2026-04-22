---
type: entity
tags: [demo, unreal-engine, arcane-demo, client-server, replication, architecture, library-separation]
---

# Maverick Demo

## What It Is
The Maverick Demo is the canonical end-to-end demonstration of the Arcane multiplayer backend, combining a Rust server (the `arcane-demo` crate and `arcane-cluster-demo` binary) with an Unreal Engine client. It serves as both a proof-of-concept for Arcane's clustering and replication capabilities and a reference integration for studios evaluating the platform.

## Origin & Evolution
The demo began as tightly coupled demo-specific logic embedded directly inside `arcane-infra`, blurring the line between infrastructure and game behavior. As the project matured, a dedicated session (2026-03-03) drove a clean architectural split: all game-specific behavior — gravity, jumping, wandering NPCs, demo agents — was extracted into a new `arcane-demo` crate. This freed `arcane-infra` to function as a pure clustering and replication library, making Arcane more credible as a general-purpose backend rather than a demo-first project.

## Technical Details
- **`arcane-demo` crate**: Houses all game-specific logic (gravity, jumping, wandering agents). Lives in the Rust workspace but is deliberately separate from `arcane-infra`.
- **`arcane-cluster-demo` binary**: Runs the full demo stack (infrastructure + demo behavior). Paired with `arcane-cluster`, the pure infrastructure binary with no demo logic.
- **`run_cluster_loop<F>` API**: Introduced to allow optional per-tick entity suppliers, enabling demo behavior to hook into the cluster loop without polluting the core library.
- **HTML viewer**: An early addition providing a way to inspect replicated state without requiring an Unreal client — useful for debugging and CI validation.
- **Unreal client**: Connects to the Rust backend via the `arcane-client-unreal` plugin. The session also addressed visual fidelity improvements (animation, client-side smoothing) and connectivity reliability.
- **Repository**: The full demo (backend + Unreal client + scripts) lives at `arcane-demos` (https://github.com/brainy-bots/arcane-demos), separate from the core library repo.

## Key Design Decisions
- **Crate separation (`arcane-demo` vs `arcane-infra`)** — Keeps the library credible as general-purpose infrastructure; studios should not need to strip out demo code to use Arcane.
- **Two binaries (`arcane-cluster` vs `arcane-cluster-demo`)** — Allows the pure library binary to be the default artifact, with the demo binary as an opt-in layer on top.
- **`run_cluster_loop<F>` with optional supplier** — The generic API boundary means demo behavior is injected, not hardcoded; the loop itself has no awareness of game logic.
- **HTML viewer for state inspection** — Decouples debugging from the Unreal client, lowering the barrier to validating replication correctness during development.
- **Separate `arcane-demos` repository** — Keeps the core library repo clean; the demo repo bundles client, server binaries, and scripts as a self-contained onboarding artifact.

## Relationships
- [[arcane-infra]]
- [[arcane-demo crate]]
- [[arcane-cluster binary]]
- [[arcane-cluster-demo binary]]
- [[run_cluster_loop API]]
- [[arcane-client-unreal]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[Replication]]
- [[HTML State Viewer]]

## Conversations That Shaped This
- [[Project documentation overview]]