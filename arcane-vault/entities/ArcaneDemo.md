---
type: entity
tags: [demo, rust, crate, arcane-demo, library-separation, clustering, replication, game-logic, binaries]
---

# ArcaneDemo

## What It Is
`arcane-demo` is a dedicated Rust crate within the Arcane workspace that houses all game-specific demo behavior — gravity, jumping, wandering agents, and demo entity logic. It exists to cleanly separate demo concerns from `arcane-infra`, allowing the core infrastructure crate to serve as a general-purpose clustering and replication library rather than a demo-first project.

## Origin & Evolution
The crate emerged from a structural problem identified during a documentation and refactoring session: `arcane-infra` had accumulated demo-specific logic (gravity, wandering, demo agents) that undermined its credibility as a general-purpose library. The solution was to extract all game-specific behavior into a standalone `arcane-demo` crate. Alongside this, a new `run_cluster_loop<F>` API was introduced to allow optional per-tick entity suppliers, and two distinct binaries were established: `arcane-cluster` (pure infrastructure, no game logic) and `arcane-cluster-demo` (demo behavior wired in). The separation also enabled an HTML viewer for inspecting replicated state without requiring an Unreal Engine client.

## Technical Details
The crate provides demo-specific entity behavior (gravity, jumping, wandering) as a pluggable layer on top of `arcane-infra`. The `run_cluster_loop<F>` generic function accepts an optional per-tick entity supplier callback, meaning demo behavior is injected at runtime rather than compiled into the infrastructure binary. The `arcane-cluster-demo` binary links `arcane-demo` into the loop, while `arcane-cluster` runs the same infrastructure path with no game logic attached. The full demo experience (backend + Unreal client + scripts) lives in the separate `arcane-demos` GitHub repository.

## Key Design Decisions
- **Dedicated crate for demo logic** — isolates game-specific code from `arcane-infra` so the library can be presented and used as general-purpose infrastructure without demo contamination
- **`run_cluster_loop<F>` API** — generic per-tick supplier hook allows demo behavior to be injected without modifying infrastructure internals, keeping the boundary clean
- **Two binaries (`arcane-cluster` / `arcane-cluster-demo`)** — explicit separation lets users of the library see a clean reference binary while still shipping a runnable demo for evaluation and testing
- **External demo repo (`arcane-demos`)** — full end-to-end demo (backend + Unreal client + scripts) lives outside the core workspace, preventing demo assets and scripts from polluting the library codebase

## Relationships
- [[ArcaneInfra]] — parent infrastructure crate that `arcane-demo` extends; game logic was extracted *from* here
- [[ArcaneCore]] — shared traits and types used by demo entities
- [[ClusterManager]] — orchestrates the cluster loop that `run_cluster_loop<F>` wraps
- [[ClusterServer]] — the runtime context in which demo entities (wandering agents, gravity simulation) execute
- [[ArcaneClientUnreal]] — Unreal Engine client that connects to `arcane-cluster-demo` to visualize demo behavior
- [[ReplicationSystem]] — demo entities' state is replicated through this system to connected clients

## Conversations That Shaped This
- [[Project documentation overview]] — primary session where the `arcane-demo` crate was conceived, the `run_cluster_loop<F>` API designed, and the binary split implemented
- [[Network library architecture review]] — established the broader principle that game logic should be cleanly separated from infrastructure, motivating the eventual extraction
- [[Untitled Chat]] — earlier visualization work that exposed the tension between demo-specific clustering behavior and library-level clustering logic