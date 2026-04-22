---
type: entity
tags: [unreal-engine, client, cpp, plugin, networking, websocket, replication, arcane-client-unreal]
---

# Unreal Engine Client

## What It Is
The Unreal Engine client is the primary game-facing consumer of the Arcane multiplayer backend, implemented as a C++ plugin (`arcane-client-unreal`) that lives in a separate repository. It connects to the Arcane cluster infrastructure over WebSocket, receives replicated entity state, and applies it to in-engine actors — while Arcane's server-side systems remain the authoritative source of truth for all simulation and game logic.

## Origin & Evolution
The client plugin emerged from a February 2026 architectural session that began with a misalignment: early setup guidance assumed an Unreal-first plugin architecture. The user corrected this, clarifying that Arcane must be a standalone backend library with Unreal as *one of several possible client targets* — not the center of the design. This pivot was foundational: it established that Arcane does not replace Unreal but rather replaces Unreal's native replication system, with Unreal reduced to a consuming client that renders and inputs but does not own simulation state.

Development environment decisions followed from this positioning. Windows was chosen as the primary build target for the plugin (over WSL) due to Unreal's deep dependency on the Windows graphics stack, DirectX, and the MSVC toolchain. WSL was retained as a useful auxiliary for CI, Linux server builds, and non-Unreal backend tooling. By early March 2026, the client work had expanded to include visual fidelity improvements — animation, client-side smoothing, and connectivity reliability — alongside architectural cleanup that separated demo-specific behavior from the core library.

## Technical Details
The plugin is added to a project's `Plugins/` folder and communicates with the Arcane backend over WebSocket. On the server side, ClusterServers handle high-frequency simulation (movement, physics, AI ticks) and push replicated state outward; the Unreal client receives that state and applies it to actors. The client does not run authoritative game logic — discrete game actions route to SpacetimeDB reducers, and the client interacts with those through the backend rather than directly. An HTML viewer was also introduced as a lightweight alternative for inspecting replicated state without requiring the Unreal editor.

The plugin's build toolchain requires MSVC Build Tools for linking compatibility with Unreal's own build pipeline. The full demo combining backend and Unreal client is maintained in the `arcane-demos` repository.

## Key Design Decisions
- **Unreal as a consuming client, not the authority** — Arcane owns simulation state; Unreal renders and inputs only, enabling engine-agnostic backend design
- **WebSocket transport** — consistent with the ClusterServer's WebSocket interface; no custom transport layer needed in the plugin
- **Windows-native development environment** — Unreal's GPU, DirectX, and MSVC dependencies make WSL an unreliable primary build target; WSL retained only for auxiliary tasks
- **Separate repository (`arcane-client-unreal`)** — keeps the plugin decoupled from the Rust workspace, allowing independent versioning and licensing treatment
- **No per-property replication flags in the wire protocol** — the four-bucket data model (Spine, Replicated, Ephemeral, Persistent) makes replication rules explicit at the type level, reducing metadata overhead on the Unreal side

## Relationships
- [[Arcane Backend]]
- [[ClusterServer]]
- [[WebSocket Transport]]
- [[SpacetimeDB Integration]]
- [[arcane-infra]]
- [[Four-Bucket Data Model]]
- [[arcane-demos]]

## Conversations That Shaped This
- [[Unreal Engine setup for networking library]] (2026-02-24)
- [[Unreal Engine networking library setup]] (2026-02-24)
- [[Untitled Chat]] (2026-02-24)
- [[Network library architecture review]] (2026-03-02)
- [[Project documentation overview]] (2026-03-03)
- [[Untitled Chat]] (2026-03-03)