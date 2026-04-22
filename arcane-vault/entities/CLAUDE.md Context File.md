---
type: entity
tags: [tooling, developer-experience, context-management, claude, ai-assisted-development, monorepo]
---

# CLAUDE.md Context File

## What It Is
A `CLAUDE.md` file is a repository-level context document placed at the root (or key subdirectories) of the Arcane codebase to bootstrap AI coding assistants — particularly Claude — with the project's architecture, conventions, and working assumptions. It serves as a persistent, machine-readable orientation guide so that every new Claude Code session starts with accurate project knowledge rather than requiring re-exploration from scratch.

## Origin & Evolution
The need for `CLAUDE.md` emerged directly from the friction observed in early Claude Code sessions, most notably the `pgp-demo` session (2026-04-16), where the assistant had to spend significant effort on orientation: reading `.gitmodules`, traversing five submodule repos, identifying stale directories on old drives, and reconstructing the project structure before any substantive work could begin. This re-exploration cost was recognized as unnecessary overhead. A `CLAUDE.md` file encodes that orientation once, so subsequent sessions can skip the archaeology and proceed directly to the task at hand. It is especially valuable in a monorepo / submodule workspace like Arcane where the repository topology is non-trivial.

## Technical Details
The file lives at the repository root and potentially in crate-level subdirectories for focused context. It typically contains:
- **Workspace layout** — the five crates (`arcane-core`, `arcane-spatial`, `arcane-rules`, `arcane-pool`, `arcane-infra`), their responsibilities, and inter-crate dependencies
- **Binary entry points** — how to run `arcane-manager` and `arcane-cluster`, feature flags required
- **Key architecture pointers** — references to `docs/SYSTEM_ARCHITECTURE.md`, `docs/MODULE_INTERACTIONS.md`, `docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md`
- **Active paths and environment** — canonical working directory, relevant drives, submodule locations to avoid stale path confusion
- **Conventions** — Rust edition, async runtime (Tokio), error-handling patterns, test conventions
- **Related repos** — `arcane-client-unreal`, `arcane-demos`, `arcane-scaling-benchmarks`, `arcane_swarm` and their roles

## Key Design Decisions
- **Placed at repo root** — Claude Code reads root-level `CLAUDE.md` automatically at session start, making orientation zero-cost for the developer
- **Human-readable prose over structured config** — the file is meant to be maintained by developers and read by both humans and AI; markdown is the natural format for the existing docs ecosystem
- **Pointers over duplication** — rather than reproducing architecture docs, `CLAUDE.md` links to authoritative sources (`SYSTEM_ARCHITECTURE.md`, etc.) to avoid drift
- **Environment anchoring** — explicitly notes canonical paths and active working directories to prevent the WSL/Windows drive confusion that surfaced in the `pgp-demo` session
- **Scoped to the monorepo reality** — acknowledges the five-submodule structure so the assistant doesn't waste cycles re-discovering it

## Relationships
- [[arcane]] — the root Rust workspace that `CLAUDE.md` contextualizes
- [[arcane-core]] — crate documented within
- [[arcane-infra]] — primary binary crate; entry points documented
- [[arcane-demos]] — related repo referenced for full demo context
- [[arcane-scaling-benchmarks]] — performance testing repo in the same workspace ecosystem
- [[arcane_swarm]] — headless load-testing engine referenced for testing context
- [[arcane-client-unreal]] — client plugin; relationship to backend documented
- [[SYSTEM_ARCHITECTURE.md]] — primary architecture doc that `CLAUDE.md` points to
- [[MODULE_INTERACTIONS.md]] — crate-level interaction reference
- [[WS_CHANNEL_BACKPRESSURE_VALIDATION.md]] — backpressure behavior doc

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]] — the orientation session that made the need for `CLAUDE.md` concrete; demonstrated the full cost of AI re-exploration in a multi-submodule workspace