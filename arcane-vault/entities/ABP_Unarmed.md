---
type: entity
tags: [unreal-engine, animation, blueprint, character, client-side, replication, demo]
---

# ABP_Unarmed

## What It Is
ABP_Unarmed is the Unreal Engine Animation Blueprint used in the Arcane demo client to drive character animations for unarmed (no weapon) locomotion and state. It translates replicated server state — position, velocity, and movement flags — into smooth, visually responsive character motion on the client side. It is a core piece of the demo's visual fidelity story, bridging the backend's replication stream and the player's on-screen experience.

## Origin & Evolution
ABP_Unarmed emerged as part of a broader effort to improve the Unreal Engine client's visual quality alongside backend architectural work. During the 2026-03-03 session, significant effort was directed at client-side smoothing and animation fidelity so that replicated entities — potentially 150–200+ concurrent networked characters — moved convincingly rather than snapping between server-authoritative positions. The animation blueprint was refined in parallel with connectivity reliability improvements and the introduction of a dedicated `arcane-demo` crate, as the project matured from a demo-first proof of concept into a more credible library with a polished showcase.

## Technical Details
ABP_Unarmed operates entirely on the Unreal Engine client (plugin: **arcane-client-unreal**). It consumes replicated entity state delivered over WebSocket from the Arcane cluster and applies client-side interpolation or smoothing before feeding the resulting velocity and movement data into the animation state machine. The blueprint handles at minimum idle, walk, and run states for unarmed characters, and is designed to work with the demo's wandering/agent entities as well as player-controlled characters. It lives in the `arcane-demos` repository alongside the full Unreal project, not inside the core Rust workspace.

## Key Design Decisions
- **Client-side only** — all animation logic stays in Unreal; the Rust backend remains engine-agnostic and never drives animation state directly, preserving Arcane's engine-neutral positioning.
- **Driven by replicated velocity/flags** — the blueprint reads interpolated position deltas rather than raw server ticks, decoupling animation smoothness from the 10 Hz server tick rate and preventing jitter at scale.
- **Unarmed scope** — keeping the blueprint scoped to unarmed locomotion simplifies the demo and avoids entangling combat or weapon systems that are not yet part of the Arcane feature set.

## Relationships
- [[arcane-client-unreal]] — the Unreal plugin that delivers replicated state to the blueprint
- [[arcane-demos]] — the repository housing the full Unreal project and this blueprint
- [[arcane-demo crate]] — the Rust-side demo crate providing wandering agents and game-specific behavior that the blueprint ultimately visualizes
- [[arcane-cluster]] — the backend binary whose replication stream feeds entity state to the client
- [[Client-side smoothing]] — the interpolation layer that makes ABP_Unarmed's input data animation-ready

## Conversations That Shaped This
- [[Project documentation overview]] (2026-03-03) — session where animation fidelity and client-side smoothing were a primary focus alongside the library/demo separation refactor
- [[Standalone binary for Unreal Engine testing]] (2026-03-06) — session targeting 150–200+ concurrent replicated characters, stress-testing the full pipeline that ABP_Unarmed sits at the end of