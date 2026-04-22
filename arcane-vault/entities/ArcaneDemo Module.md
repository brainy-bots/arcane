---
type: entity
tags: [demo, visualization, integration, unreal-engine, clustering, benchmarks]
---

# ArcaneDemo Module

## What It Is
ArcaneDemo (repo: `arcane-demos`) is the official demonstration and reference integration layer for the Arcane multiplayer backend. It provides a full working example combining the Arcane backend with an Unreal Engine client and supporting scripts, serving as both a developer onboarding tool and a living proof-of-concept for Arcane's clustering, replication, and session management capabilities.

## Origin & Evolution
The demo was created to lower the barrier to entry for studios evaluating Arcane — the core library documents its architecture extensively, but seeing the system behave end-to-end (backend binaries + client plugin + scripted load) required a separate, self-contained artifact. The demo repo was split from the main workspace deliberately to keep the library crate clean of demo-specific dependencies and to allow the demo to evolve at its own pace. A significant design evolution emerged in a 2026-02-24 session: early demo visualizations used naive spatial proximity as the clustering signal, which produced unrealistic, oscillating cluster behavior. This was replaced with interaction-likelihood metrics (guild membership, party relationships, enemy state) combined with hysteresis thresholds to prevent merge/split oscillation. Server load was added as a scaling signal so that player convergence triggers new server spawning rather than cluster collapse.

## Technical Details
The demo is housed at `https://github.com/brainy-bots/arcane-demos` and is referenced from the main Arcane README as the recommended starting point for running a full system. It exercises the two reference server binaries shipped in `arcane-infra`:
- **`arcane-manager`** — HTTP join endpoint (the `manager` feature flag)
- **`arcane-cluster`** — WebSocket + Redis server (the `cluster-ws` feature flag)

The Unreal Engine client component is drawn from the **arcane-client-unreal** plugin (separate repo), which is added to the Unreal project's `Plugins/` folder. The demo also includes scripts (load generation, visualization) used to exercise and observe the backend's dynamic clustering behavior. The clustering visualization evolved to simulate behavioral metrics rather than positional proximity, with hysteresis thresholds as the key stabilization mechanism.

## Key Design Decisions
- **Separate repo from core library** — keeps `arcane` workspace free of demo dependencies and allows independent versioning and iteration
- **Interaction-likelihood over spatial proximity** — clustering decisions reflect who players are likely to interact with (guild, party, enemy state), not just where they stand, producing more realistic and stable cluster topologies
- **Hysteresis thresholds** — prevent the merge/split oscillation that naive threshold-based clustering produces; clusters must cross a meaningful boundary before topology changes are committed
- **Server load as a scaling signal** — player convergence triggers new server spawning rather than overloading a single cluster instance
- **GIF/scripted output over GitHub Gist** — visualization output was found too complex for Gist format; richer formats (GIF, scripts) are used instead for communicating system behavior

## Relationships
- [[arcane-infra]] — provides the `arcane-manager` and `arcane-cluster` binaries the demo runs
- [[arcane-client-unreal]] — Unreal Engine plugin consumed by the demo client project
- [[ClusterManager]] — the backend component whose behavior the demo most visibly exercises
- [[RulesEngine]] — clustering decisions (interaction-likelihood, hysteresis) that the demo's visualization exposes
- [[arcane-core]] — foundational traits and types underlying all demo interactions

## Conversations That Shaped This
- [[Untitled Chat (2026-02-24)]] — clustering visualization evolution from spatial proximity to behavioral metrics; hysteresis design; Unreal Engine integration issues encountered during demo development