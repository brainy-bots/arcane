---
type: entity
tags: [demo, architecture, arcane-demo, library-separation, unreal-engine, replication, clustering]
---

# DEMO_GOAL.md

## What It Is
The Demo Goal is the guiding objective for Arcane's reference demo: produce a compelling, end-to-end showcase of the multiplayer backend's clustering, replication, and physics simulation capabilities without conflating demo-specific logic with the library itself. It serves as both a credibility signal for potential adopters and a live integration test of the full stack — from `arcane-infra` through the Unreal Engine client.

## Origin & Evolution
The demo began as a mixed-concern codebase where game-specific behavior (gravity, jumping, wandering agents) lived inside `arcane-infra`, blurring the line between library and demonstration. The core problem this created was a perception risk: Arcane looked like a demo-first project rather than a general-purpose multiplayer backend library. The 2026-03-03 session drove a structural resolution — creating a dedicated `arcane-demo` crate to own all game-specific logic, freeing `arcane-infra` to serve purely as infrastructure. A new `run_cluster_loop<F>` API was introduced to allow optional per-tick entity suppliers, making the separation clean at the binary level as well.

## Technical Details
The demo is hosted in the `arcane-demo` crate and produces a dedicated binary (`arcane-cluster-demo`) distinct from the pure-infrastructure binary (`arcane-cluster`). Game-specific behaviors — gravity, jumping, wandering agents, demo entity spawning — are supplied to the cluster loop via the `run_cluster_loop<F>` callback API rather than being embedded in core infrastructure. The Unreal Engine client connects via WebSocket and renders replicated state; an HTML viewer was also developed to inspect replicated state without requiring Unreal. The full demo repository lives at [arcane-demos](https://github.com/brainy-bots/arcane-demos) and combines the backend, Unreal client plugin, and supporting scripts.

## Key Design Decisions
- **`arcane-demo` as a separate crate** — isolates all game-specific logic so `arcane-infra` remains a credible general-purpose library with no demo contamination
- **Two binaries (`arcane-cluster` vs `arcane-cluster-demo`)** — allows users to run pure infrastructure without pulling in demo behavior, while the demo binary demonstrates the full feature set
- **`run_cluster_loop<F>` per-tick supplier API** — provides an extension point for demo (or user) entity logic without requiring changes to core infrastructure
- **HTML viewer as a secondary client** — lowers the barrier to inspecting replicated state during development and demos, independent of the Unreal Engine setup
- **Demo hosted in a separate repository (`arcane-demos`)** — keeps the core library repo clean and signals that the demo is an *example consumer* of Arcane, not the product itself

## Relationships
- [[arcane-demo]] — the crate housing all demo-specific game logic
- [[arcane-infra]] — the pure infrastructure crate the demo consumes
- [[run_cluster_loop]] — the API boundary between infrastructure and demo-supplied entity logic
- [[arcane-cluster-demo]] — the demo binary produced by the separation
- [[arcane-client-unreal]] — the Unreal Engine client plugin used in the demo
- [[arcane-demos]] — the external repository containing the full demo stack
- [[SYSTEM_ARCHITECTURE.md]] — system-level diagrams showing how the demo fits the full topology

## Conversations That Shaped This
- [[Project documentation overview]] (2026-03-03) — the session that drove the library/demo separation, introduced `arcane-demo` crate, `run_cluster_loop<F>`, and dual-binary structure