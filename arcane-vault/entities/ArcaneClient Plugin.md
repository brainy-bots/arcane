---
type: entity
tags: [unreal-engine, client, plugin, websocket, sdk, integration]
---

# ArcaneClient Plugin

## What It Is
The ArcaneClient Plugin (`arcane-client-unreal`) is the Unreal Engine client-side integration for the Arcane multiplayer backend. It lives in a separate repository and is added to a project's `Plugins/` folder, providing the client-facing SDK that handles communication with the Arcane cluster infrastructure over WebSocket. It is the bridge between an Unreal Engine game client and the Arcane server ecosystem.

## Origin & Evolution
The plugin emerged as the natural client-side complement to the Arcane Rust library — the recognition that the backend system needed a first-class Unreal integration to be usable in practice, without forcing studios to write their own WebSocket plumbing. A key design principle from early in the project was engine agnosticism on the server side (Unity, Unreal, Godot, or custom engines can all use Arcane), but Unreal was the first and primary client target given its dominance in high-fidelity game development. Integration with the plugin surfaced cascading build issues during a session focused on Unreal Engine integration, indicating active development and the expected friction of bridging Rust backend infrastructure with Unreal's build toolchain (UBT/C++ module system).

## Technical Details
- Distributed as a separate repository (`arcane-client-unreal`), added to a game project's `Plugins/` directory following standard Unreal plugin conventions.
- Communicates with the Arcane backend via WebSocket — the same transport layer used by `arcane-infra`'s `arcane-cluster` binary, which exposes a WebSocket endpoint alongside Redis for replication.
- The join flow begins with an HTTP request to the `arcane-manager` binary (the Manager component), after which the client connects via WebSocket to the assigned ClusterServer.
- Referenced alongside `arcane-demos` (a companion demo repo) as the end-to-end validation path: backend + Unreal client + scripts forming the full integration test surface.
- Build issues encountered during integration sessions suggest active C++ module boundary work, likely around wrapping async WebSocket handling in a way compatible with Unreal's game thread model.

## Key Design Decisions
- **Separate repository** — keeps the Unreal plugin (C++, UBT, proprietary toolchain) fully decoupled from the Rust workspace, allowing independent versioning and avoiding contaminating the Rust build system.
- **WebSocket as the client transport** — consistent with the cluster server's primary interface; no custom protocol is needed on the client side beyond what `arcane-infra` already exposes.
- **Engine-agnostic server, Unreal-first client** — the backend makes no assumptions about client engine, but the Unreal plugin is the reference implementation; other engine clients would follow the same HTTP-join + WebSocket pattern.
- **Plugin folder convention** — using Unreal's standard `Plugins/` integration path means studios adopt it without modifying engine source, lowering integration friction.

## Relationships
- [[arcane-infra]] — the Rust crate providing the ClusterManager and ClusterServer binaries the plugin connects to
- [[arcane-manager]] — the HTTP join endpoint the client first contacts to receive a server assignment
- [[arcane-cluster]] — the WebSocket server the plugin maintains its live session connection with
- [[arcane-demos]] — the companion demo repository pairing the plugin with a running backend for end-to-end validation
- [[WebSocket Transport]] — the communication layer between plugin and cluster
- [[ClusterManager]] — orchestrates which ClusterServer the client is routed to

## Conversations That Shaped This
- [[Untitled Chat (2026-02-24)]] — surfaced cascading Unreal Engine integration and build issues during active plugin development; context for the build toolchain friction encountered when bridging Rust infrastructure with UBT