---
type: entity
tags: [toolchain, unreal-engine, windows, development-environment, build-system, arcane-client-unreal]
---

# Visual Studio Toolchain

## What It Is
The Visual Studio Toolchain refers to the suite of Microsoft developer tools — Visual Studio IDE, MSVC compiler, IntelliSense, and the associated debugging and build pipeline infrastructure — used as the primary development environment for the `arcane-client-unreal` Unreal Engine plugin. It is the required toolchain for building, iterating on, and debugging UE5 plugins on Windows, and represents a deliberate platform choice made early in the Arcane client development.

## Origin & Evolution
The toolchain decision emerged during the 2026-02-24 session on Unreal Engine networking library setup, when the team was establishing a development environment for what would become `arcane-client-unreal`. The central question at the time was whether to develop natively on Windows or within an existing WSL (Ubuntu) environment. Visual Studio on Windows was chosen as the primary environment after evaluating the tradeoffs: Unreal Engine's editor is GPU-intensive and tightly coupled to the Windows graphics stack (DirectX, native drivers), and UE's own build pipeline, plugin scaffolding, and documentation are all designed and validated Windows-first. Fighting WSL for interactive editor and plugin work was assessed as introducing unnecessary friction at every iteration cycle.

## Technical Details
Visual Studio serves as the authoritative build environment for the UE plugin layer of Arcane. Key integration points include:
- **MSVC compiler**: Required for UE5 plugin compilation; UE's Unreal Build Tool (UBT) invokes MSVC directly and expects the Windows SDK and associated headers to be present.
- **IntelliSense**: UE generates `.sln` and `.vcxproj` files to drive IDE code intelligence; Visual Studio's IntelliSense engine consumes these for navigation and error highlighting in plugin code.
- **Debugger**: Native Windows debugger integration allows attaching to the Unreal Editor process for real-time plugin debugging, which is not reliably achievable from WSL.
- **Plugin build pipeline**: UE's plugin packaging and cooking workflows are invoked through Visual Studio project targets or `RunUAT.bat`, both requiring the full Windows toolchain.

WSL remains in use for non-UE concerns: building Linux-target dedicated server binaries, running CI/automation scripts, and all Rust workspace (`arcane-core`, `arcane-infra`, etc.) development.

## Key Design Decisions
- **Windows-native over WSL for UE work** — WSL lacks reliable GPU passthrough and DirectX integration needed by the Unreal Editor; native Windows eliminates an entire class of environment-mismatch bugs.
- **Visual Studio as primary IDE (not VS Code or Rider for initial setup)** — UBT's `.sln`/`.vcxproj` generation targets Visual Studio directly; using it avoids configuration overhead at project start.
- **WSL retained as auxiliary environment** — Clean separation of concerns: Windows/Visual Studio owns UE plugin work; WSL owns Rust backend, Linux server builds, and CI tooling. Avoids a monolithic platform dependency.
- **Toolchain decision made before first plugin line** — Resolving environment friction early prevented it from compounding across plugin iteration cycles.

## Relationships
- [[arcane-client-unreal]]
- [[Unreal Engine Plugin Architecture]]
- [[WSL Development Environment]]
- [[Windows vs WSL Tradeoffs]]
- [[Unreal Build Tool (UBT)]]
- [[arcane-infra]]

## Conversations That Shaped This
- [[Unreal Engine networking library setup]]