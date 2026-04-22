---
type: conversation
date: 2026-03-06
source: cursor
tags: [arcane, spacetimedb, unreal-engine, benchmarking, rust, multiplayer, clustering, redis, aws, docker, animation, replication, scaling]
---

# Standalone binary for Unreal Engine testing

**Date:** 2026-03-06
**Source:** cursor (277671 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/cursor-chat-export-pgp-demo/2026-03-06-12-Standalone_binary_for_Unreal_E.md`

## Summary

This session was a comprehensive, multi-phase engineering effort spanning the full stack of the Arcane multiplayer backend project. The primary goal was to prove Arcane's distributed cluster architecture outperforms a SpacetimeDB-only backend at scale, while simultaneously building a production-quality Unreal Engine demo that showcases entity replication across 150–200+ concurrent networked characters.

The session began with establishing a fair benchmarking methodology: building a headless Rust "swarm" binary (`arcane-swarm`) that simulates real game clients using the actual SpacetimeDB SDK (WebSocket + BSATN + subscriptions), replacing earlier HTTP REST polling approaches that unfairly penalized SpacetimeDB. Canonical workload parameters were locked down (10 Hz tick rate, 2 actions/sec, 30-second runs, spread movement, everyone-sees-everyone visibility) to enable apples-to-apples comparisons. Key findings: SpacetimeDB-only ceiling is ~1000 concurrent players (with server-side physics via `physics_tick` scheduled reducer); Arcane+SpacetimeDB scales from ~1250 (1 cluster) to ~5500 players (10 clusters), with the fundamental advantage being O(N) serialization vs SpacetimeDB's O(N²) subscription fanout.

On the Unreal Engine side, the session resolved a series of interconnected problems around entity visibility, coordinate system alignment, animation pipeline ordering, and movement replication. The final architecture uses a plug-and-play library design: the `ArcaneAdapterSubsystem::ApplyEntityStateToActor` API feeds external server state into Unreal's standard replicated-movement pipeline, allowing the same `ACharacter` class and Animation Blueprint used by the player to drive all entities without custom subsystems. A separate `UArcaneEntityMovementSyncComponent` ticking in PostUpdateWork re-applies velocity/acceleration before the skeletal mesh evaluates, preventing the CharacterMovementComponent from overwriting externally-set values.

The infrastructure work culminated in three significant deliverables: (1) a standalone `arcane-scaling-benchmarks` repository with vendored swarm runtime and SpacetimeDB module, eliminating all `arcane-demos` dependencies; (2) a Docker Compose v2 containerized benchmark profile with per-service resource limits and published GHCR images; and (3) an AWS EC2 cloud orchestration pipeline using SSM for remote execution and S3 for artifact persistence. The repository split (arcane library, arcane-client-unreal plugin, arcane-demos game) was also completed during this session, establishing clean versioning boundaries.

## What Was Built

- **`arcane-swarm` binary** — headless Rust client simulator using SpacetimeDB SDK (WebSocket + BSATN + subscriptions) supporting both SpacetimeDB-only and Arcane cluster backends, with configurable player count, tick rate, movement patterns, and TCP control interface (`SET_PLAYERS`, `RESET`, `REPORT`, `QUIT`)
- **SpacetimeDB module** — `Entity` table (position-only), private `PlayerInput` table, scheduled `physics_tick` reducer, `set_entities` batch persist reducer, B-tree indexes on spatial columns
- **`arcane-scaling-benchmarks` repository** — self-contained public benchmark repo with vendored swarm runtime, vendored SpacetimeDB module source, canonical parameter documentation, and PowerShell orchestration scripts
- **Ceiling sweep scripts** — `Run-SpacetimeDBCeilingSweep.ps1`, `Run-ArcaneScalingSweep.ps1`, `Run-Benchmark-Scenarios.ps1` (incremental, control-mode), `Run-FullBenchmark-Incremental.ps1`
- **Docker Compose v2 benchmark** — containerized stack with resource limits, `docker-compose.v2.yml`, `docker-compose.v2.repro.yml`, published GHCR images (`arcane-benchmark-infra`, `arcane-benchmark-swarm`, `arcane-benchmark-runner`)
- **AWS cloud orchestration** — `Run-Benchmark-V2-Aws.ps1`, EC2+SSM+S3 pipeline, minimal Terraform for durable S3 bucket, GitHub Actions workflow for image publishing
- **`ApplyEntityStateToActor` API** — plugin function applying position/velocity/rotation via `FRepMovement` path for simulated proxies or direct transform for Authority actors in standalone
- **`UArcaneEntityMovementSyncComponent`** — PostUpdateWork tick component re-applying velocity/acceleration before ABP evaluation
- **`ABP_ArcaneEntity`** — duplicate of `ABP_Unarmed` with "Is Locally Controlled" gates removed from Event Graph
- **Dual networking mode** — GameMode toggle between Arcane (Rust backend) and default Unreal replication (`AReplicatedBotSpawner`) using identical character class
- **Verification loop** — Python `capture_game.py` + PowerShell runner with PrintWindow API, input blocking, window management, and multi-cluster variants
- **`ASpacetimeDBEntityDisplay`** — Unreal C++ actor using SpacetimeDB Unreal SDK, subscribing to Entity table and applying rows via `ApplyEntityStateToActor`
- **Repository split** — arcane (Rust library), arcane-client-unreal (plugin), arcane-demos (demo game + scripts), meta-repo with submodules
- **System architecture documentation** — `SYSTEM_ARCHITECTURE.md` with Mermaid diagrams, `CANONICAL_BENCHMARK_PARAMS.md`, `SCALING_EXPERIMENT_RESULTS.md`, `BENCHMARK_V2_METHOD.md`, `CLOUD_BENCHMARK_AWS.md`

## Key Decisions

- **Separate SpacetimeDB concerns**: relegated SpacetimeDB to low-frequency batch persistence (1 Hz `set_entities`) while Arcane clusters own real-time physics and state distribution, removing N² subscription fanout from the hot path
- **Server-side physics pattern**: moved from client-driven position updates to private `PlayerInput` table + scheduled `physics_tick` reducer, reducing subscription waves from 2N² to N² per tick and improving SpacetimeDB ceiling from ~150 to ~1000 players
- **No spatial subscriptions**: dynamic region resubscription (every ~1.25s) created worse overhead than full-table `SELECT * FROM entity`; dropped for the benchmark baseline
- **Uncapped batch persistence**: single large HTTP POST per persist window outperforms many small capped batches (500 entities/request) under canonical workload
- **CMC tick disabled for entities**: prevents `CalcVelocity()` from overwriting externally-set velocity; display component + sync component take full ownership of velocity/acceleration lifecycle
- **Plug-and-play library contract**: plugin only replaces networking; same `ACharacter`, `ACharacterMovementComponent`, and ABP as player; no custom entity types required from customers
- **Vendored benchmark runtime**: copied swarm code and SpacetimeDB module into `arcane-scaling-benchmarks` repo to eliminate `arcane-demos` dependency and enable public reproducibility without credentials
- **Published GHCR images with minimal EC2 bootstrap**: runner image contains full toolchain so EC2 instances only install Docker + git, dramatically reducing cold-start cost on metered compute
- **Separate infra vs ephemeral**: Terraform manages durable S3 bucket; PowerShell scripts handle ephemeral EC2 lifecycle per run
- **`BackendRuntime` trait**: decouples swarm orchestration from backend-specific protocol details (Arcane vs SpacetimeDB), enabling clean repo separation

## Problems Solved

- **SpacetimeDB N² subscription fanout** — identified as root cause of ~150 player ceiling; resolved architecturally by moving physics to Arcane clusters and using SpacetimeDB only for batch persistence
- **HTTP polling unfairly penalized SpacetimeDB** — entire benchmark rewritten to use native SDK with WebSocket + BSATN + subscriptions
- **Entities "flying" above player** — entity world origin locked at spawn height before player landed; fixed by deferring origin placement 2 seconds until player has touched ground
- **Entities forming vertical line** — server sent `(x, y, z)` with y=height; client mapped y→Unreal Y (horizontal); fixed by server sending `(a.x, a.z, a.y)` so third component is always vertical
- **Animations stuck in T-pose/idle** — `ABP_Unarmed` Event Graph only updates Speed when "Is Locally Controlled"; fixed by creating `ABP_ArcaneEntity` that always reads velocity without possession check
- **Velocity overwritten by CMC each tick** — `CalcVelocity()` zeroed externally-set velocity; fixed by disabling CMC tick and adding PostUpdateWork sync component
- **Authority actors in standalone ignored position updates** — `OnRep_ReplicatedMovement` only fires for SimulatedProxy; fixed by branching on role and applying transform directly for Authority
- **Port conflicts killing multi-cluster tests** — stale processes left 8081/8091 bound; manager fallback routed all traffic to one cluster; fixed with pre-run cleanup, pre-flight verification, and removal of hardcoded fallback ports
- **UUID serialization in JSON** — `u128` values over 2^53 can't be serialized as JSON numbers; fixed by constructing raw JSON strings inline
- **SpacetimeDB CLI not on PATH** — scripts now detect default install location and prepend to PATH automatically
- **Resubscription churn worse than fanout** — spatial subscriptions with 750-unit threshold caused 160 resubscribes/sec per 200 players; reverted to full-table subscription
- **Humanoid meshes invisible in PIE** — material/render-state issues; fixed with deferred mesh setup to `BeginPlay`, engine material override in editor builds, `MarkRenderStateDirty()` calls
- **AWS EC2 cold bootstrap cost** — full Rust/SpacetimeDB toolchain install per run took 45+ minutes; moved toolchain into `arcane-benchmark-runner` Docker image, EC2 now only installs Docker + git
- **SSM buffering hiding failures** — added `--output-s3-bucket-name` so full transcript goes to S3; local script polls for heartbeats only
- **Windows PowerShell JSON mangling** — `cmd /c` stripped quotes from inline AWS CLI JSON; fixed by writing JSON to temp files and passing `file://` URIs
- **GHCR packages private** — org policy blocked anonymous pull; resolved by updating org package creation settings to allow public packages
- **wasm-opt missing lowering SpacetimeDB ceilings** — added `Install-WasmOpt.ps1` and prerequisite detection warnings
- **Error accounting undercounting failures** — action-path errors not included in `FINAL total_errs`; fixed to compute effective error rate across both write/read and action call totals

## Entities

- [[Arcane Engine]]
- [[PGP Architecture]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[arcane_swarm]]
- [[SpaceTimeDB]]
- [[Unreal Engine Client]]
- [[Benchmark System]]
- [[AWS Infrastructure]]
- [[Rapier]]
- [[Redis]]
- [[Spatial Grid]]
- [[arcane-scaling-benchmarks]]
- [[arcane-demos]]
- [[CI Pipeline]]
- [[arcane-client-unreal]]

NEW:
- NEW: [[arcane-benchmark-swarm]]
- NEW: [[ABP_ArcaneEntity]]
- NEW: [[UArcaneEntityMovementSyncComponent]]
- NEW: [[ASpacetimeDBEntityDisplay]]
- NEW: [[ArcaneAdapterSubsystem]]
- NEW: [[physics_tick reducer]]
- NEW: [[GHCR Benchmark Images]]
- NEW: [[Benchmark Journal