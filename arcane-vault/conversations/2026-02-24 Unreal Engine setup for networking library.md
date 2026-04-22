---
type: conversation
date: 2026-02-24
source: cursor
tags: [unreal-engine, networking, cpp, standalone-library, architecture, setup, powershell, msvc, replication, toolchain]
---

# Unreal Engine setup for networking library

**Date:** 2026-02-24
**Source:** cursor (120 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/cursor-chat-export-pgp-demo/2026-02-24-17-Unreal_Engine_setup_for_networ.md`

## Summary

This session established the initial development environment and architectural foundation for a standalone C++ networking library intended to integrate with — but remain independent from — Unreal Engine. The core goal was not to build an Unreal plugin, but to design a library that *replaces* Unreal's native replication system by acting as the authoritative backend, with Unreal serving purely as a consuming client.

The session began with a misalignment: early setup guidance assumed an Unreal-first plugin architecture. The user corrected this, clarifying the library must be standalone, with Unreal as one of several possible client targets. This triggered a pivot from plugin-focused toolchain configuration to a leaner, toolchain-only setup — MSVC Build Tools for linking compatibility, without full Visual Studio IDE.

The primary architectural pattern that emerged is a four-interface design: `IClusteringModel`, `IServerPool`, `IReplicationChannel`, and `IWorldSimulator`. Each interface accepts pluggable implementations (e.g., `StaticRules` vs ML-driven clustering; `LocalPool` vs ECS-backed server pools; TCP vs in-process replication channels; static, fast-forward, or ML-driven world simulation). This keeps the core library free of Unreal types while enabling clean integration at the boundary.

The session concluded with a validated architecture, an automated PowerShell setup script, and a clear implementation order. Open questions remained around whether cluster servers would be Unreal instances or separate processes, and whether a C API shim layer would be required in the first implementation slice.

## What Was Built

- `scripts/Setup-UnrealEnvironment.ps1` — PowerShell script using `winget` to automate installation of Epic Games Launcher and Visual Studio 2022 Build Tools (C++ workload only), with iterative path and elevation fixes applied during the session
- `config/unreal-env.json` — configuration file tracking install status and paths for Epic Launcher and Build Tools across runs
- `docs/ARCHITECTURE_REVIEW_AND_IMPLEMENTATION.md` — architecture review document covering the four-interface design pattern and a phased implementation roadmap
- Conceptual design of the four-interface system: `IClusteringModel`, `IServerPool`, `IReplicationChannel`, `IWorldSimulator` with named pluggable implementations for each

## Key Decisions

- **Standalone-first architecture**: The library has no hard dependency on Unreal Engine; Unreal is a client that consumes entity state deltas from the replication channel, not a host for the library's logic
- **No Visual Studio IDE**: Only MSVC Build Tools installed to satisfy linker requirements when targeting Unreal; development proceeds in VS Code, CLion, or equivalent
- **Four-interface pluggable design**: Each major subsystem (clustering, server pool, replication, simulation) is defined as a pure interface with swappable implementations, enabling testing and deployment without Unreal present
- **Unreal as replication consumer**: Unreal receives state updates through `IReplicationChannel` rather than using its native networking stack — this is the mechanism by which the library "replaces" Unreal networking without coupling to Unreal types
- **Scripted setup over static docs**: Shifted from a written environment guide to an automated PowerShell script with JSON state tracking to reduce manual error during toolchain installation
- **Phased implementation order**: Types/interfaces → StaticRules + LocalPool → TCP channel → Static/FastForward simulator → optional C API → Unreal integration plugin

## Problems Solved

- **Architecture misalignment**: Initial assumption that the library was Unreal-plugin-centric; corrected to standalone library model after user clarification, reframing the entire setup and design approach
- **Script path errors**: `Setup-UnrealEnvironment.ps1` initially pointed to incorrect root directory paths; corrected during iterative runs
- **Elevation handling**: Removed admin elevation requirement from the script itself; elevation prompts are now delegated to `winget` and individual installers as needed, avoiding unnecessary privilege escalation
- **Scope of toolchain install**: Narrowed from full Visual Studio IDE to Build Tools only (C++ workload), reducing install footprint while satisfying MSVC linker compatibility for Unreal integration

## Entities

- [[Arcane Engine]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[Unreal Engine Client]]
- [[arcane-client-unreal]]
- [[arcane-demos]]

NEW entities:
- NEW: [[IClusteringModel]] — pure C++ interface for clustering decisions; accepts `StaticRules` or ML-driven implementations
- NEW: [[IServerPool]] — pure C++ interface for server pool management; accepts `LocalPool` or ECS-backed implementations
- NEW: [[IReplicationChannel]] — pure C++ interface for state delta distribution; accepts TCP or in-process implementations; the integration boundary between the library and Unreal Engine Client
- NEW: [[IWorldSimulator]] — pure C++ interface for world simulation; accepts static, fast-forward, or ML-driven implementations
- NEW: [[Setup-UnrealEnvironment.ps1]] — PowerShell automation script for MSVC Build Tools and Epic Games Launcher installation

## Related Conversations

_to be linked_