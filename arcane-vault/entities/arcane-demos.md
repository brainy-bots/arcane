---
type: entity
tags: [demo, repository, unreal-engine, integration, benchmarking, client, documentation]
---

# arcane-demos

## What It Is
`arcane-demos` is a companion repository (hosted at `https://github.com/brainy-bots/arcane-demos`) that provides a full end-to-end demonstration of the Arcane multiplayer backend. It bundles the backend infrastructure, Unreal Engine client, and supporting scripts into a single runnable showcase, serving as both a proof-of-concept and the primary integration reference for developers adopting Arcane.

## Origin & Evolution
The need for a dedicated demos repo emerged from a structural tension: early development embedded demo-specific logic (gravity, jumping, wandering agents, demo entity suppliers) directly inside `arcane-infra`, muddying its identity as a general-purpose library. A refactor session (2026-03-03) created a dedicated `arcane-demo` crate to house all game-specific behavior, freeing `arcane-infra` to be a clean clustering and replication library. A new `run_cluster_loop<F>` API was introduced to support optional per-tick entity suppliers, and two binaries were split: `arcane-cluster` (pure infrastructure) and `arcane-cluster-demo` (demo behavior). The `arcane-demos` repo became the external home for the full integrated experience — backend, Unreal client plugin, and scripts — while the main `arcane` workspace remained library-focused and credible as a standalone dependency.

## Technical Details
The repo combines three layers:
1. **Backend** — runs the Arcane cluster and manager binaries, demonstrating WebSocket replication and Redis-backed state across 150–200+ concurrent simulated characters.
2. **Unreal Engine client** — an integration of the `arcane-client-unreal` plugin, showcasing entity replication with mannequin characters, client-side smoothing, and animation. Debugging work (2026-03-06) resolved dynamic material interference that prevented mannequins from rendering; the fix involved stripping problematic dynamic material logic to restore visibility on build.
3. **Scripts** — supporting tooling for running the demo stack, likely including swarm simulation scripts derived from the `arcane-swarm` headless Rust binary built to simulate real game clients at scale.

An HTML viewer for inspecting replicated state without Unreal was also developed as part of the demo layer, providing a lightweight alternative for observing cluster state during development.

## Key Design Decisions
- **Separation of demo logic from library** — Game-specific behavior was extracted into `arcane-demo` crate and `arcane-cluster-demo` binary so that `arcane-infra` remains a credible general-purpose library rather than a demo-first project.
- **`run_cluster_loop<F>` API** — Introduced to allow optional per-tick entity suppliers, making demo behavior opt-in rather than baked into the cluster loop.
- **External repo rather than workspace crate** — Keeping the full Unreal + backend + scripts bundle outside the main Rust workspace avoids coupling the library's release cycle to demo assets and engine-specific files.
- **HTML state viewer** — Added as a low-friction inspection tool so developers can observe replicated state without requiring a full Unreal Engine setup.

## Relationships
- [[arcane-infra]]
- [[arcane-core]]
- [[arcane-client-unreal]]
- [[arcane-cluster binary]]
- [[arcane-manager binary]]
- [[arcane-swarm]]
- [[ClusterServer]]
- [[ClusterManager]]
- [[SpacetimeDB]]
- [[Redis]]

## Conversations That Shaped This
- [[Project documentation overview]] (2026-03-03) — primary session where demo/library separation was designed and the `arcane-demos` repo structure was established
- [[Standalone binary for Unreal Engine testing]] (2026-03-06) — benchmarking session that validated the demo at 150–200+ concurrent entities and finalized the swarm tooling included in the repo
- [[Project repository status]] (2026-03-06) — debugging session resolving the mannequin rendering issue in the Unreal client component of the demo
- [[Network library architecture review]] (2026-03-02) — architecture decisions that determined what belongs in the library vs. the demo layer
- [[Untitled Chat]] (2026-02-24) — early visualization work and clustering behavioral simulation that informed demo design