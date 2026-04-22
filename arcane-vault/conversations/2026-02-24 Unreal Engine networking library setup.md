---
type: conversation
date: 2026-02-24
source: cursor
tags: [unreal-engine, networking, development-environment, windows, wsl, arcane-client-unreal, setup]
---

# Unreal Engine networking library setup

**Date:** 2026-02-24
**Source:** cursor (13 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/cursor-chat-export-pgp-demo/2026-02-24-19-Unreal_Engine_networking_libra.md`

## Summary
This session focused on establishing the right development environment for building a networking library plugin for Unreal Engine — the work that would become `arcane-client-unreal`. The central question was whether to develop natively on Windows or inside a WSL (Ubuntu) environment, given the user was already operating in WSL.

After evaluating the tradeoffs, Windows was chosen as the primary development environment. The core reasoning was that Unreal Engine's editor is GPU-intensive and deeply integrated with the Windows graphics stack (DirectX, native drivers), making WSL an unreliable foundation for interactive editor work. The toolchain — Visual Studio, IntelliSense, debugging, plugin build pipelines — is also designed and documented Windows-first, so fighting WSL would introduce unnecessary friction at every iteration.

WSL was not dismissed entirely. It was retained as a useful auxiliary environment for building and testing Linux-target dedicated server builds, running CI pipelines and automation scripts, and supporting non-Unreal backend tooling work on the same machine. The distinction drawn was: WSL is excellent for scripting-friendly, headless, Linux-native workflows; Unreal plugin development is none of those things.

The outcome of the session was a clear, documented environmental baseline — Windows primary, WSL auxiliary — that positions the team to iterate on the networking library without hitting GPU passthrough instability, missing toolchain integrations, or documentation gaps that plague WSL-based Unreal workflows.

## What Was Built
- Development environment decision document (Windows-primary, WSL-auxiliary strategy)
- Clear role separation between Windows (Unreal editor, plugin dev, VS toolchain) and WSL (Linux builds, CI, backend scripts)

## Key Decisions
- **Windows as primary Unreal development environment** — Unreal Editor is GPU-heavy and Windows-first; using WSL introduces GPU passthrough latency via WSLg, toolchain mismatches, and documentation gaps
- **WSL retained for Linux-specific auxiliary tasks** — dedicated server builds, Linux game exports, CI automation, and backend/tools work can still run in WSL without friction
- **Avoid mixed-environment iteration loops** — the interactive Unreal editor workflow must run natively; keeping it on Windows prevents compounding environment-related friction during plugin development

## Problems Solved
- **Environment mismatch identified early** — user was already in WSL Ubuntu; session clarified upfront that this would be suboptimal before any plugin scaffolding was attempted, avoiding wasted setup effort
- **WSL GPU passthrough concerns addressed** — WSLg GPU passthrough latency and stability risks for an editor-heavy workload were surfaced and used to justify the Windows decision
- **Toolchain integration gaps resolved** — Visual Studio + Unreal IntelliSense, debugging, and build pipeline compatibility firmly land on Windows, not WSL

## Entities
- [[arcane-client-unreal]]
- [[Unreal Engine Client]]
- [[CI Pipeline]]
- [[arcane-scaling-benchmarks]]

Also list any NEW entities not in the seed (prefix with NEW:):
- NEW: [[WSL (Windows Subsystem for Linux)]]
- NEW: [[Visual Studio Toolchain]]

## Related Conversations
_to be linked_