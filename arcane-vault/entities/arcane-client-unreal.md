---
type: entity
tags: [unreal-engine, client, plugin, cpp, networking, replication, websocket, arcane]
---

# arcane-client-unreal

## What It Is
`arcane-client-unreal` is the Unreal Engine client plugin for the Arcane multiplayer backend, living in a separate repository from the core Rust library. It connects Unreal Engine games to the Arcane cluster infrastructure — handling WebSocket communication, entity replication, and character state — while deliberately *replacing* Unreal's native replication system rather than layering on top of it. Studios add it to their project's `Plugins/` folder to gain Arcane-backed multiplayer without being locked into Unreal's own networking stack.

## Origin & Evolution
The plugin emerged from a February 2026 session that began with a misalignment: early setup guidance assumed a standard Unreal-first plugin architecture, and the developer corrected course, clarifying that the library must be standalone with Unreal as one of several possible client targets. This triggered a pivot away from plugin-centric toolchain configuration toward a leaner design where the backend is authoritative and Unreal is purely a consumer. A subsequent session resolved the development environment question — Windows was chosen as the primary platform over WSL, due to Unreal's deep dependency on DirectX, Windows-native GPU drivers, and a toolchain (Visual Studio, IntelliSense, plugin build pipelines) that is documented and supported Windows-first. WSL was retained as an auxiliary environment for Linux server builds and CI. By March 2026 the plugin was far enough along to power a demo showcasing 150–200+ concurrent networked characters, and a mannequin rendering bug caused by problematic dynamic material logic was debugged and resolved, resulting in a working build.

## Technical Details
The plugin is an Unreal Engine C++ plugin added via the project's `Plugins/` folder. It communicates with the Arcane backend over WebSocket, receiving replicated entity state from `ClusterServer` instances and persistent game state routed through SpacetimeDB. The architectural boundary is clear: game logic and authoritative simulation live server-side (in ClusterServers and SpacetimeDB reducers respectively); the Unreal client renders and inputs only. The plugin interfaces with the four-interface backend design — `IClusteringModel`, `IServerPool`, `IReplicationChannel`, and `IWorldSimulator` — treating Unreal as the presentation layer for state produced by those abstractions. A headless Rust swarm binary (`arcane-swarm`) exists as a parallel test client, used for benchmarking and load testing the same backend endpoints the Unreal plugin connects to, ensuring the plugin's wire protocol is not the only path exercised.

## Key Design Decisions
- **Standalone library, not an Unreal-native plugin** — the backend must be engine-agnostic; Unreal is one client target among several, so the core networking logic is not coupled to Unreal's replication primitives
- **Replaces Unreal's native replication system** — rather than wrapping or augmenting Unreal Net, the plugin delegates authority entirely to the Arcane cluster, avoiding conflicts between two replication systems
- **Windows as primary dev environment** — WSL ruled out for interactive editor work due to GPU/DirectX requirements and toolchain friction; WSL retained only for Linux server builds and CI
- **Dynamic material logic stripped from character setup** — a rendering bug with mannequin visibility was traced to dynamic material initialization interfering with mesh visibility; the fix was to bypass that logic rather than patch it, prioritizing a clean working build
- **Separate repository from arcane core** — keeps the Rust workspace free of Unreal build system entanglement and allows the plugin to version independently

## Relationships
- [[arcane-core]] — provides the traits and shared types the plugin's backend counterpart implements
- [[arcane-infra]] — runs `arcane-cluster` (WebSocket) and `arcane-manager` (HTTP join) that the plugin connects to
- [[arcane-swarm]] — headless Rust client that exercises the same backend endpoints, used for benchmarking
- [[spacetimedb]] — authoritative persistent state source; plugin may subscribe to SpacetimeDB tables for game actions and persistent state
- [[arcane-demos]] — companion demo repository containing the full backend + Unreal client integration example

## Conversations That Shaped This
- [[Unreal Engine setup for networking library]] — established standalone-library architecture and four-interface backend design
- [[Unreal Engine networking library setup]] — resolved Windows vs WSL dev environment question
- [[Untitled Chat]] — first Unreal integration issues surfaced alongside clustering visualization work
- [[Network library architecture review]] — clarified game-logic-in-SpacetimeDB boundary that defines what the client does and does not own
- [[Standalone binary for Unreal Engine testing]] — demo built to 150–200+ concurrent characters; arcane-swarm validated backend at scale
- [[Project repository status]] — mannequin rendering bug debugged and resolved; working build confirmed