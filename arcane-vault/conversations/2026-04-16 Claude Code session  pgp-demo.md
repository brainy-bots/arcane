---
type: conversation
date: 2026-04-16
source: claude-code
tags: [arcane, benchmarks, aws, docker, spacetimedb, rust, architecture, infrastructure, harness, wire-protocol, licensing]
---

# Claude Code session — pgp-demo

**Date:** 2026-04-16
**Source:** claude-code (3 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/claude-conversations/mnt-e-code-pgp-demo/ab0c3fbc-9b49-4b30-a35a-5b308a2c247a.jsonl`

## Summary

This was a sprawling, multi-phase session covering the full arc from library hardening through benchmark infrastructure, live AWS runs, and harness refactoring for the Arcane multiplayer backend project. The session began with merging four PRs into the core `arcane` library (entity state model, simulation trait, input validation, licensing) and publishing v0.1.0. Simultaneously, a fundamental flaw in the benchmark design was identified and corrected: SpacetimeDB-only mode ran real physics while Arcane mode ran none, making comparisons meaningless. A five-connection-type architecture was defined and three new benchmark crates were created to produce equivalent workloads across both modes.

The second major phase focused on AWS infrastructure and live benchmark execution. The pipeline was migrated from on-EC2 compilation to pre-built Docker images pulled from GHCR, dramatically reducing runtime cost and complexity. First production benchmark runs were executed, establishing SpacetimeDB-only ceiling at ~250–500 players and Arcane ceiling at ~2000 players — but a critical measurement bug was uncovered: Arcane's `lat_avg_ms` measured only local WebSocket TX buffer write time (~0.01 ms) rather than real round-trip latency, making the Arcane ceiling meaningless. The session also introduced binary wire protocol via a new `arcane-wire` crate using postcard serialization (3.4× faster encode, 2.3× smaller payload vs JSON), driver CPU/memory telemetry, and released arcane v0.3.0 and arcane_swarm v0.2.0.

The final phase addressed deep harness architecture problems. The benchmark script assumed a single topology could run both SpacetimeDB-only and Arcane scenarios sequentially, but they require mutually exclusive WASM module deployments. After an initial false start splitting into two entry scripts, the correct design settled on a single `Run-Benchmark.ps1` dispatcher driven by a `BenchmarkMode` config field. Config files were converted to JSONC format with inline documentation for every parameter. Stale code (unused functions, dead parameters), silent-failure risks from missing config fields, and stale manifest repro commands were all cleaned up. Terraform was established as the canonical infrastructure management tool, with PowerShell scoped to run/collect phases only.

A cross-cutting concern throughout: driver-side measurement integrity. At high player counts, the swarm driver on a single EC2 instance may be CPU-bound on parse/deserialize rather than actually saturating the server. The honest fix is multi-swarm distribution (task #29) so the server still sees N concurrent clients while driver CPU is spread across M machines. Driver telemetry (CPU + RSS sampling) was added as a first step to distinguish driver bottleneck from server bottleneck.

## What Was Built

- **arcane v0.1.0–v0.3.0**: Four-bucket entity state model (`user_data`/`local_data`), `ClusterSimulation` trait + `simulate_before_tick()` hook, `GameAction` type, input validation (NaN/Infinity rejection, size caps, entity limits), WebSocket `GAME_ACTION` message routing, binary wire protocol via `arcane-wire` crate (postcard encoding)
- **arcane_swarm v0.1.0–v0.2.0**: Cluster action routing, binary-only WebSocket sends
- **Three new benchmark crates**: `benchmark-spacetimedb-full` (SpacetimeDB-only with physics/collision/buffs), `benchmark-spacetimedb-persist` (Arcane-mode persistence only), `benchmark-cluster` (Arcane cluster with `BenchmarkSimulation` trait)
- **Single multi-stage Dockerfile** baking all binaries + WASM modules, published to GHCR as `ghcr.io/brainy-bots/arcane-benchmark:<tag>`
- **Rewritten cloud scripts**: `RemoteBenchmark.ps1` heredocs rewritten for `docker pull && docker run`; `-BenchmarkImage` parameter added; Terraform user-data stripped to Docker + AWS CLI only
- **Terraform module** committed at `infra/terraform/aws_benchmark/` as canonical infrastructure tool
- **`arcane-wire` crate**: typed encode/decode helpers (`encode_client`, `decode_client`, `encode_server`, `decode_server`), minimal dependencies (serde + postcard + uuid)
- **Driver telemetry**: CPU and RSS sampling from `/proc/self/stat`, per-second prints + CSV columns
- **Refactored harness**: single `scripts/Run-Benchmark.ps1` with `-ConfigFile` dispatcher; `ConvertFrom-BenchmarkConfigJsonc` comment-stripping helper; `Assert-RequiredScriptVariablesSet` guard; two JSONC config files (`spacetimedb_only.json`, `arcane_plus_spacetimedb.clusters_2.json`)
- **`connection-types.md`** architecture doc explaining five connection patterns and developer decision guide
- **AGPL-3.0 licensing** applied across five repos (arcane, arcane_swarm, arcane-scaling-benchmarks, arcane-demos, arcane-client-unreal)

## Key Decisions

- **Five-connection-type architecture**: Client↔Cluster (WebSocket, simulation-affecting), Cluster↔Cluster (Redis pub/sub, replication), Cluster↔SpacetimeDB (HTTP, persistence + action validation), Client↔SpacetimeDB direct (cosmetic/transactional), optional SpacetimeDB→Cluster subscriptions for slow state. Developer picks path per action based on simulation impact.
- **Workload parity requirement**: Both benchmark modes must run identical O(n²) collision algorithm with identical physics constants (WORLD_SIZE=5000, PHYSICS_SPEED=600, COLLISION_RADIUS=50) — the whole point is a fair comparison.
- **Docker image pipeline**: Pre-build everything into a single image; EC2 instances only pull and run. Eliminates per-run compile costs and ensures reproducibility.
- **Terraform-first infrastructure**: Terraform owns provision/destroy; PowerShell scoped to run/collect phases only. PowerShell setup/cleanup scripts to be deleted.
- **Binary wire protocol (postcard)**: Dual-path WebSocket in cluster accepts both JSON (Text frames) and binary (postcard) per-connection; swarm sends binary exclusively. 3.4× faster encode, 2.3× smaller than JSON.
- **Single harness entry point with config dispatch**: Martin rejected two entry scripts. `BenchmarkMode` in config JSON drives which scenario runs; all workload knobs live in config, not param block. JSONC format for inline documentation.
- **Honest latency measurement**: Arcane's `lat_avg_ms` must measure actual round-trip, not TX buffer write time. Until fixed, Arcane ceiling numbers are not meaningful saturation measurements.
- **Multi-swarm distribution for scaling claims**: Keeping 1 connection per simulated player is non-negotiable for fair server load measurement. Driver CPU bottleneck addressed by distributing swarm across M machines (task #29), not by reducing connection count.
- **AGPL-3.0 licensing**: Copyright under Juan Martín Mingo Suárez personally pending company incorporation; CLA infrastructure deferred to post-incorporation.

## Problems Solved

- **Unfair benchmark comparison**: SpacetimeDB mode running physics, Arcane mode running none — resolved by creating equivalent simulation crates for both modes with identical algorithms and constants.
- **SpacetimeDB HTTP body format bug**: `[[{uuid},args]]` → `[{uuid},args]` (wrong nesting level)
- **Velocity double-application**: Cluster was re-multiplying pre-computed displacement
- **Collision damage performance**: Changed from 2 HTTP calls per collision to batched reducer calls
- **EC2 build cost**: Migrated from on-instance compilation to pre-built GHCR images
- **Broken Arcane latency semantics**: Identified that `lat_avg_ms` measured TX buffer write (~0.01 ms) not round-trip; ceiling detection was therefore never finding real saturation
- **Mutually exclusive WASM modules**: SpacetimeDB-only needs `benchmark-spacetimedb-full`; Arcane needs `benchmark-spacetimedb-persist`. A single topology can't run both. Resolved by splitting into separate config-driven scenarios within a single dispatcher script.
- **Silent config failures**: Missing fields from initial refactor caused `$null` reads with no error. Resolved with `Assert-RequiredScriptVariablesSet` guard that enumerates missing fields before any side effects.
- **Stale harness interfaces**: Removed unused `Get-ArcaneClustersEntitiesTotal`, dead `-FindArcaneCeiling` parameter, stale repro command flags in manifest output.
- **Uncommitted Terraform module**: Was billing 8 EC2 instances from ad-hoc PowerShell provisioning; Terraform module committed and PowerShell setup scripts slated for deletion.

## Entities

- [[Arcane Engine]]
- [[PGP Architecture]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[arcane_swarm]]
- [[SpaceTimeDB]]
- [[Benchmark System]]
- [[AWS Infrastructure]]
- [[Redis]]
- [[arcane-scaling-benchmarks]]
- [[arcane-demos]]
- [[arcane-client-unreal]]
- [[CI Pipeline]]
- [[Unreal Engine Client]]

NEW entities:
- NEW: [[arcane-wire]] — minimal postcard/serde binary wire protocol crate; typed encode/decode helpers for WebSocket messages
- NEW: [[benchmark-spacetimedb-full]] — SpacetimeDB-only benchmark crate with full server-side physics, collision, and buff simulation
- NEW: [[benchmark-spacetimedb-persist]] — Arcane-mode SpacetimeDB benchmark crate; persistence only, no physics reducers
- NEW: [[benchmark-cluster]] — Arcane cluster benchmark crate implementing `BenchmarkSimulation` trait with kinematic physics
- NEW: [[BenchmarkSimulation]] — `ClusterSimulation` trait implementation used in benchmark-cluster; mirrors SpacetimeDB physics_tick logic exactly
- NEW: [[Five Connection Types]] — architectural pattern defining which operations route through which path (Client-Cluster WS, Cluster-Cluster Redis, Cluster-SpacetimeDB HTTP, Client-SpacetimeDB direct, subscriptions)
- NEW: [[GHCR Benchmark Image]] — pre-built Docker image (`ghcr.io/brainy-bots/arcane-benchmark:<tag>`) containing all binaries and WASM modules for AWS benchmark runs

## Related Conversations

_to be linked_