---
type: entity
tags: [powershell, toolchain, setup, unreal-engine, msvc, development-environment, scripting, automation]
---

# Setup-UnrealEnvironment.ps1

## What It Is
`Setup-UnrealEnvironment.ps1` is a PowerShell automation script that configures the Windows development toolchain for building the Arcane networking library with Unreal Engine linking compatibility. It installs and configures MSVC Build Tools — the compiler and linker required for C++ library compatibility with Unreal — without requiring a full Visual Studio IDE installation. It exists to make the correct toolchain setup reproducible and unambiguous for developers entering the Arcane ecosystem.

## Origin & Evolution
The script emerged from a foundational tension uncovered in the February 2026 setup session: early guidance assumed an Unreal-first, plugin-style architecture, which implied a full Visual Studio + Unreal Engine plugin toolchain. When the project's actual goal was clarified — a **standalone** Rust/C++ networking library that treats Unreal as one of several client targets — the toolchain requirements were reframed. The full IDE was unnecessary; only MSVC Build Tools were needed for linking compatibility between the Arcane library and Unreal's runtime. The script was the concrete output of that pivot: a lean, reproducible way to install just what is needed, nothing more.

## Technical Details
The script targets Windows and automates the installation and configuration of MSVC Build Tools, which provide the C++ compiler (`cl.exe`) and linker (`link.exe`) that Unreal Engine requires when consuming native libraries. Because Arcane is a standalone library (not an Unreal plugin), the script does not install the Unreal Engine itself or a full Visual Studio IDE — it installs only the build toolchain components necessary for ABI and linking compatibility. This reflects the architectural principle that Unreal is a **consumer** of Arcane, not its host: the library must compile independently, and the toolchain setup must not create coupling to any single engine's build system.

## Key Design Decisions
- **MSVC Build Tools only, not full Visual Studio** — rationale: Arcane is not an Unreal plugin; the library must remain engine-agnostic, and pulling in a full IDE would imply an Unreal-first coupling that contradicts the project's multi-client architecture.
- **PowerShell scripting for Windows toolchain** — rationale: MSVC is a Windows-native toolchain; PowerShell is the idiomatic automation layer on Windows for developer environment setup, making the script accessible and maintainable without external tooling dependencies.
- **Reproducibility over manual steps** — rationale: the session that produced this script identified toolchain misconfiguration as a likely friction point for new contributors; automating it removes ambiguity about which MSVC components are required.

## Relationships
- [[arcane-client-unreal]] — the Unreal Engine client plugin this toolchain enables building against
- [[IClusteringModel]] — one of the four core interfaces of the standalone library the toolchain is set up to compile
- [[IReplicationChannel]] — core interface; Unreal's replication system is replaced by Arcane via this interface
- [[arcane-infra]] — the Rust crate housing the ClusterManager and replication logic that the Unreal client consumes

## Conversations That Shaped This
- [[Unreal Engine setup for networking library]] — the founding session; established the standalone-library architecture, identified the MSVC-only toolchain requirement, and produced the script as the resolution to the plugin-vs-library misalignment