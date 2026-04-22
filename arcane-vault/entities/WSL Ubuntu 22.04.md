---
type: entity
tags: [environment, devops, wsl, ubuntu, windows, tooling, infrastructure]
---

# WSL Ubuntu 22.04

## What It Is
WSL Ubuntu 22.04 (Windows Subsystem for Linux) is the primary local development environment used for Arcane backend work on Windows machines. It provides a Linux shell environment where Rust builds, cargo commands, and shell scripts execute natively, bridging the gap between Windows as a host OS and the Linux-native toolchain that Arcane's Rust codebase expects.

## Origin & Evolution
WSL Ubuntu 22.04 emerged as the practical solution for developing Arcane on Windows without dual-booting or maintaining a separate Linux machine. The environment became notable in the project record during the `pgp-demo` session (2026-04-18), where it was the active shell context for a non-trivial environment setup task: installing PowerShell 7.6.0 to enable scripting tooling that pgp-demo depended on. That session exposed two friction points specific to the WSL environment — Microsoft packages not being available in Ubuntu's default apt repositories, and multi-line paste behavior stripping newlines and corrupting pasted scripts.

## Technical Details
WSL Ubuntu 22.04 runs as a Linux userspace layer on top of Windows, exposing a bash/sh environment where standard Linux package management (`apt`, `dpkg`) applies. The environment accesses the Windows filesystem under `/mnt/` (e.g., `/mnt/e/code/`), which is where project files from Windows drives are mounted. Rust toolchains, cargo, and shell scripts all run in this Linux context. The PowerShell installation incident revealed a specific constraint: Microsoft's PowerShell package requires manual registration of Microsoft's apt repository and GPG signing key (`dpkg` of the `.deb` repo config, then `apt-get update`) before `apt-get install powershell` succeeds. A persistent terminal quirk in this environment is that pasting multi-line bash scripts collapses newlines, turning separate commands into a single malformed command string; the workaround is to chain commands with `&&` or `;` on a single line before pasting.

## Key Design Decisions
- **WSL over native Windows toolchain** — Arcane's Rust workspace and shell-based workflows assume Linux conventions; WSL provides this without requiring a dedicated Linux machine or VM.
- **Manual Microsoft apt repo registration** — PowerShell and other Microsoft tools are not in Ubuntu's default repos; the project established the pattern of registering Microsoft's repo via `.deb` config before installing such packages.
- **Single-line command chaining as paste workaround** — Multi-line scripts cannot be reliably pasted into WSL terminals; chaining with `&&` or `;` became the practical convention for ad-hoc setup scripts in this environment.

## Relationships
- [[pgp-demo]] — the project context in which WSL Ubuntu 22.04 environment issues were encountered and resolved
- [[PowerShell 7.6.0]] — the tool that triggered the Microsoft apt repo setup problem in this environment
- [[arcane-scaling-benchmarks]] — the workspace directory (`/home/vr0n1n/Workspace/arcane-scaling-benchmarks/`) where session artifacts are stored, implying benchmarking work also runs in this environment

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]] (2026-04-18) — the session that surfaced WSL-specific installation and paste-behavior issues, establishing the workarounds documented here