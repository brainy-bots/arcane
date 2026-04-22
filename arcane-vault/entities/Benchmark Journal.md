---
type: entity
tags: [benchmarks, arcane, spacetimedb, aws, ci-pipeline, rust, infrastructure, performance]
---

# Benchmark Journal

## What It Is
The Benchmark Journal is the living record of performance comparisons between Arcane's distributed cluster architecture and competing backends (primarily SpacetimeDB). It captures canonical workload definitions, key findings, infrastructure decisions, and result artifacts produced by the `arcane-scaling-benchmarks` repository. It serves as the evidentiary foundation for Arcane's core claim: that its multi-server clustering approach outperforms single-process backends at scale.

## Origin & Evolution
The journal grew out of an early need to prove Arcane's distributed architecture delivers measurable, reproducible advantages. Initial benchmarks used HTTP REST polling against SpacetimeDB, which was quickly identified as an unfair methodology. A pivotal milestone was replacing that approach with a headless Rust swarm binary (`arcane-swarm`) that uses the actual SpacetimeDB SDK (WebSocket + BSATN + subscriptions), making comparisons apples-to-apples. Canonical workload parameters were then locked down — 10 Hz tick rate, 2 actions/sec, 30-second runs, spread movement, everyone-sees-everyone visibility — so results would be reproducible across runs and infrastructure. Subsequent sessions hardened CI, fixed AWS infrastructure bugs, and clarified the separation of provisioning, execution, and teardown scripts.

## Technical Details
The benchmark harness lives in `arcane-scaling-benchmarks` and is organized around a JSON state contract shared between the PowerShell CLI provisioning path and the Terraform IaC path. Key components:

- **`arcane-swarm`**: headless Rust binary that simulates real game clients; uses SpacetimeDB SDK for fair load generation
- **PowerShell scripts**: `Setup-AwsBenchmark.ps1` (provisions, writes state JSON), `Run-Benchmark-AwsRemote.ps1` (reads state, delegates to harness), `Run-Benchmark.ps1` (assumes infra exists, runs workload only), `Cleanup-AwsBenchmark.ps1` (teardown)
- **Directory layout**: `infra/aws/` with `lib/` (shared helpers), `topologies/` (environment profiles), JSON benchmark configuration profiles
- **CI**: GitHub Actions pipeline with Pester test suite; historically broken by Pester v3/v4 vs v5 syntax incompatibility, Node.js deprecation warnings, SSM timeout misconfiguration, and missing submodule auth
- **Key finding logged**: SpacetimeDB-only ceiling ~1,000 concurrent players with server-side physics via `physics_tick` scheduled reducer; Arcane cluster architecture targets beyond that threshold

## Key Design Decisions
- **Replace HTTP polling with real SDK swarm client** — earlier REST polling unfairly penalized SpacetimeDB; the swarm binary uses the actual WebSocket + BSATN path for honest comparison
- **Lock canonical workload parameters early** — prevents benchmark drift and enables reproducible comparisons across infrastructure changes and time
- **Separate provisioning, execution, and teardown** — `Run-Benchmark.ps1` must not embed infrastructure provisioning; each script has a single responsibility tied to the JSON state contract
- **Coexist PowerShell CLI and Terraform paths** — both converge on the same state JSON contract, allowing teams to use either provisioning method without changing the run harness
- **Pester v5 syntax throughout** — legacy `Should` assertions removed after CI breakage; all tests written to v5 conventions

## Relationships
- [[arcane-swarm]]
- [[SpacetimeDB]]
- [[ClusterManager]]
- [[arcane-infra]]
- [[arcane-scaling-benchmarks]]
- [[AWS Infrastructure]]
- [[CI Pipeline]]
- [[Canonical Workload Parameters]]

## Conversations That Shaped This
- [[Standalone binary for Unreal Engine testing]]
- [[CI pipeline failure in Arcane Scaling Benchmarks]]
- [[Benchmark improvement suggestions]]
- [[Project directory exploration and analysis]]