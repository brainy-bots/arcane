---
type: entity
tags: [unreal-engine, demo, client, character, replication, animation, arcane-demos]
---

# AArcaneDemoCharacter

## What It Is
`AArcaneDemoCharacter` is the Unreal Engine character class used in the Arcane demo project (`arcane-demos`). It serves as the client-side representation of replicated entities received from the Arcane backend, handling movement, animation, and visual smoothing for remote-controlled characters in the demo scene.

## Origin & Evolution
The character class emerged as part of the `arcane-demos` companion repository, which provides a full end-to-end demonstration of the Arcane backend paired with an Unreal Engine client. During the 2026-03-03 session, the demo underwent a significant architectural split: game-specific behavior (gravity, jumping, wandering agents) was moved out of `arcane-infra` and into a dedicated `arcane-demo` Rust crate, which meant the Unreal client's character class needed to reliably consume a cleaner, more stable replication stream. Effort in that session also went into improving visual fidelity and connectivity reliability on the Unreal side, directly affecting how `AArcaneDemoCharacter` interpolates and displays remote state.

## Technical Details
- Lives in the `arcane-demos` repository (separate from the core Rust workspace), added to an Unreal project under `Plugins/` via **arcane-client-unreal**.
- Receives replicated entity state broadcast by the `arcane-cluster` or `arcane-cluster-demo` binary over WebSocket.
- Implements client-side smoothing/interpolation to handle the tick-rate mismatch between the backend replication loop and Unreal's render loop.
- Works alongside an HTML viewer path (also introduced in the same session) that can inspect replicated state without requiring Unreal, confirming the replication stream is engine-agnostic.
- Animation state is driven by velocity/state data decoded from backend replication messages rather than local physics prediction.

## Key Design Decisions
- **Client-side smoothing over raw position snapping** — rationale: tick-rate mismatches between the Rust backend and Unreal's frame loop cause visible jitter without interpolation; smoothing was prioritized during the 2026-03-03 visual fidelity pass.
- **Character is demo-only, not part of arcane-client-unreal core** — rationale: keeping game-specific character logic in `arcane-demos` preserves `arcane-client-unreal` as a generic plugin, consistent with the library-vs-demo separation enforced on the Rust side.
- **Driven by replicated state, not authoritative local physics** — rationale: the Arcane backend owns simulation (gravity, jumping, wandering); the character class is a pure display consumer, matching the server-authoritative design of the cluster.

## Relationships
- [[arcane-demos]] — repository where this character class lives
- [[arcane-client-unreal]] — Unreal plugin that provides the WebSocket transport and replication decoding the character consumes
- [[arcane-demo crate]] — Rust crate supplying the game-specific entity behavior (gravity, jumping, wandering) that this character class visualizes
- [[arcane-cluster-demo binary]] — the backend binary whose replication stream this character consumes
- [[ClusterManager]] — orchestrates the cluster servers that ultimately push state to this client
- [[ReplicationStream]] — the data pipeline between backend and this character

## Conversations That Shaped This
- [[Project documentation overview]] — 2026-03-03 session covering the library/demo architectural split and the Unreal visual fidelity and connectivity reliability improvements that directly shaped this class