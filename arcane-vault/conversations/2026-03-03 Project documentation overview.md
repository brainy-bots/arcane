---
type: conversation
date: 2026-03-03
source: cursor
tags: [arcane, multiplayer-backend, rust, unreal-engine, demo, replication, clustering, animation, client-side-smoothing, library-architecture, documentation]
---

# Project documentation overview

**Date:** 2026-03-03
**Source:** cursor (2783 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/cursor-chat-export-pgp-demo/2026-03-03-14-Project_documentation_overview.md`

## Summary

This session was a comprehensive overhaul of the Arcane multiplayer backend demo, focused on two parallel tracks: cleanly separating library concerns from demo-specific logic in the Rust workspace, and improving the Unreal Engine client's visual fidelity and connectivity reliability. The session began with work on an HTML viewer for inspecting replicated state without requiring Unreal, and evolved into a broader architectural refactor.

The primary structural outcome was the creation of a dedicated `arcane-demo` crate to house all game-specific behavior — gravity, jumping, wandering, demo agents — freeing `arcane-infra` to serve as a pure clustering and replication library. A new `run_cluster_loop<F>` API was introduced to allow optional per-tick entity suppliers, and two binaries were established: `arcane-cluster` for pure infrastructure and `arcane-cluster-demo` for demo behavior. This separation makes Arcane more credible as a general-purpose library rather than a demo-first project.

On the Unreal side, significant effort went into visual alignment between the playable character and replicated demo agents. Client-side interpolation was implemented to smooth entity movement between server snapshots (0.1s delay window), and mesh handling was standardized on Quinn/Manny mannequin assets from the Unreal template content. The `ABP_Unarmed` animation blueprint was integrated for both the playable character and replicated entities, with `AM_Unarmed_Jog` triggered when entity speed exceeds threshold.

A phased roadmap was established for evolving the demo toward a Maverick-style MMO showcase: (1) align visuals between player and agents, (2) scale and measure performance, (3) implement server-authority handoff, (4) add persistence via SpacetimeDB, and (5) push entity count limits. Diagnostic logging was added throughout the adapter subsystem and entity display layer to assist with ongoing multi-cluster connectivity debugging.

## What Was Built

- **`arcane-demo` crate** — new crate housing all game/demo logic (gravity, jump, wander, demo agents), separated from infrastructure
- **`arcane-cluster-demo` binary** — demo-specific cluster binary using the new crate
- **`run_cluster_loop<F>` API** — new `arcane-infra` function accepting optional per-tick entity suppliers
- **`AArcaneDemoCharacter`** — third-person playable character (WASD + mouse, jump, spring-arm camera)
- **Client-side interpolation** — frame-by-frame smooth interpolation between server state snapshots with configurable 0.1s delay
- **Ground alignment property** — configurable `GroundZ` in entity display to map server coordinate space to Unreal floor height
- **Humanoid mesh fallback** — capsule + sphere fallback when skeletal mesh is unavailable
- **`Copy-CharacterFromTemplate.ps1`** — PowerShell script to migrate character content from Third Person template
- **`DEMO_GOAL.md`** — document framing the demo as a library capability showcase with capability/production-usage mapping table and phased roadmap
- **Diagnostic logging** — comprehensive logging in `ArcaneAdapterSubsystem` (join, WebSocket, state updates, parse failures, zero-entity warnings) and `ArcaneEntityDisplay` (BeginPlay, auto-connect, adapter availability)
- **HTML viewer** — minimal HTML client fetching `/join` and WebSocket-connecting to cluster for state inspection without Unreal
- Updated build scripts and crate READMEs reflecting library/demo split

## Key Decisions

- **Hard separation of library vs. demo code**: `arcane-infra` must not contain game logic; all demo behavior moves to `arcane-demo` crate — rationale: makes Arcane credible as a general-purpose library and allows users to build their own game logic on top
- **`run_cluster_loop<F>` over embedded agents**: entity suppliers are injected as a function parameter rather than compiled into the library — rationale: keeps the core loop generic and composable
- **Standardize on Quinn/Manny template assets**: rather than custom meshes, use assets already available in UE5 Third Person template — rationale: lowers barrier to entry for developers evaluating the library
- **Single animation blueprint for all entities**: both playable character and replicated agents use `ABP_Unarmed` — rationale: visual consistency for demo showcase; makes the crowd feel like real players
- **Phased demo roadmap**: incremental milestones (visual alignment → scale → authority → persistence → limits) rather than big-bang approach — rationale: each phase produces a demonstrable, shareable milestone
- **`DEMO_GOAL.md` as explicit framing document**: document the demo's purpose as library showcase, not a game — rationale: prevents scope creep and aligns contributors on intent

## Problems Solved

- **Library/demo coupling**: `arcane-infra` contained embedded demo agents making it look like a game engine rather than a library — resolved by extracting `arcane-demo` crate
- **Snapping entity movement**: replicated entities teleported between server ticks rather than moving smoothly — resolved with client-side interpolation buffer
- **Coordinate space mismatch**: server ground at y=0 did not align with Unreal floor height — resolved with configurable `GroundZ` property
- **Missing skeletal mesh handling**: entities failed visually when mannequin assets were unavailable — resolved with capsule+sphere humanoid fallback
- **Connectivity debugging opacity**: multi-cluster WebSocket issues were difficult to diagnose — resolved with structured diagnostic logging covering join, connect, state update, and zero-entity warning paths
- **Content availability in demo project**: mannequin and animation assets not present in demo project — resolved by migrating from Third Person template via `Copy-CharacterFromTemplate.ps1` and merging UEIntroProject content

## Entities

- [[Arcane Engine]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[arcane_swarm]]
- [[SpaceTimeDB]]
- [[Unreal Engine Client]]
- [[Redis]]
- [[arcane-demos]]

NEW entities:
- NEW: [[arcane-demo crate]] — Rust crate housing all demo/game-specific behavior separated from `arcane-infra`
- NEW: [[AArcaneDemoCharacter]] — Unreal Engine third-person playable character actor for the Arcane demo
- NEW: [[ArcaneAdapterSubsystem]] — Unreal subsystem managing WebSocket connection, join requests, and entity state updates
- NEW: [[ArcaneEntityDisplay]] — Unreal actor responsible for spawning and updating visual representations of replicated entities
- NEW: [[DEMO_GOAL.md]] — Documentation artifact framing the demo as a library capability showcase with phased roadmap
- NEW: [[run_cluster_loop]] — Generic API function in `arcane-infra` accepting optional per-tick entity suppliers
- NEW: [[ABP_Unarmed]] — Unreal animation blueprint used for both playable character and replicated entities
- NEW: [[Maverick Demo]] — Reference MMO-style demo concept used as the scaling and capability target for the Arcane showcase

## Related Conversations

_to be linked_