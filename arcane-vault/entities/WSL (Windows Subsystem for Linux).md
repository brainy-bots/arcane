---
type: entity
tags: [development-environment, windows, wsl, linux, toolchain, arcane-client-unreal, infrastructure]
---

# WSL (Windows Subsystem for Linux)

## What It Is
WSL (Windows Subsystem for Linux) is Microsoft's compatibility layer for running Linux environments natively on Windows. In the Arcane project, WSL serves as a secondary development environment on Windows machines — useful for backend tooling and Linux-target builds, but explicitly not the primary environment for Unreal Engine plugin work.

## Origin & Evolution
The WSL question arose during the February 2026 session on setting up the `arcane-client-unreal` development environment. The developer was already operating inside a WSL (Ubuntu) environment and needed to determine whether to stay there or move to native Windows for Unreal Engine plugin development. After evaluating the tradeoffs, WSL was retained in a supporting role rather than dismissed outright — a pragmatic compromise that acknowledged both its utility and its limitations.

## Technical Details
The core issue with WSL for Unreal Engine work is GPU access and graphics stack integration. Unreal's editor is GPU-intensive and deeply coupled to Windows-native DirectX and driver infrastructure; WSL's graphics passthrough is insufficient for reliable interactive editor sessions. Additionally, the Unreal toolchain — Visual Studio, IntelliSense, plugin build pipelines, and debugging — is designed and documented Windows-first, meaning WSL introduces friction at every iteration of the plugin development loop.

Where WSL does work well in the Arcane context:
- Building and testing Linux-target dedicated server builds
- Running CI pipelines and automation scripts
- Backend tooling work (Rust, arcane-infra binaries) on the same machine as Unreal development

## Key Design Decisions
- **Windows chosen over WSL for Unreal plugin work** — GPU integration, DirectX dependency, and toolchain documentation all point to native Windows; WSL would introduce unnecessary friction at every iteration
- **WSL retained as auxiliary environment** — not dismissed, but scoped to non-Unreal tasks: Linux server builds, CI/automation, and backend tooling where it is fully capable
- **Clear boundary drawn between WSL and Windows workloads** — this prevents environment confusion and keeps the plugin development loop clean

## Relationships
- [[arcane-client-unreal]] — the Unreal Engine client plugin whose development environment decision drove the WSL evaluation
- [[arcane-infra]] — backend binaries that can be built and run inside WSL on the same machine
- [[Development Environment]] — the broader setup question WSL was evaluated within

## Conversations That Shaped This
- [[Unreal Engine networking library setup]] — the session where WSL vs. native Windows was evaluated and the boundary between the two environments was established