---
type: entity
tags: [aws, infrastructure, topology, benchmark, spacetimedb, deployment, testing]
---

# AwsSpacetimeOnly Topology

## What It Is
`AwsSpacetimeOnly` is one of the named AWS benchmark topologies in the `arcane-scaling-benchmarks` project. It provisions an environment consisting solely of a SpacetimeDB node — no Arcane cluster servers — and is used to benchmark SpacetimeDB in isolation as a direct comparison baseline against Arcane's own cluster topologies.

## Origin & Evolution
This topology emerged from the broader benchmark infrastructure restructuring documented in the 2026-03-30 session. During that session the old `scripts/cloud/environments/` directory was renamed to `infra/aws/topologies/`, and individual topology definitions (including `AwsSpacetimeOnly`) were formalized as discrete, named configurations rather than ad-hoc scripts. The separation of concerns between provisioning (`Setup-AwsBenchmark.ps1`), running (`Run-Benchmark-AwsRemote.ps1`), and teardown (`Cleanup-AwsBenchmark.ps1`) made topology-specific state (written as JSON by setup and consumed by the run harness) the natural home for topology identity — and `AwsSpacetimeOnly` became a first-class member of that set. Its purpose is to answer the direct competitive question: how does SpacetimeDB alone perform at the same workloads Arcane is benchmarked against?

## Technical Details
- Lives under `infra/aws/topologies/` after the directory restructure from `scripts/cloud/environments/`.
- `Setup-AwsBenchmark.ps1` provisions the topology and writes a state JSON file; `Run-Benchmark-AwsRemote.ps1` reads that state file to know which hosts exist and what roles they play.
- In `AwsSpacetimeOnly`, the state JSON describes a single SpacetimeDB host with no Arcane `ClusterServer` or `ClusterManager` nodes.
- The run harness must not inject Arcane-specific multi-host parameters when operating against this topology; multi-host Arcane parameters that had leaked into the run harness were explicitly identified for removal during the same session.
- All import paths, CI workflow references, and Pester test assertions were updated to the new `infra/aws/topologies/` path, so CI pipelines referencing this topology by name continue to resolve correctly.

## Key Design Decisions
- **Topology as a first-class concept** — Moving from `environments/` to `topologies/` and giving each a stable name (e.g. `AwsSpacetimeOnly`, `AwsArcaneCluster`) lets the run harness branch on topology identity rather than inferring it from ad-hoc flags.
- **State JSON as the contract** — Provisioning writes JSON; running reads it. This decoupling means `AwsSpacetimeOnly` can be set up once and re-run many times without re-provisioning, and teardown can use the same state to know exactly what to destroy.
- **No Arcane parameters injected** — When the active topology is `AwsSpacetimeOnly`, multi-host Arcane cluster parameters are explicitly excluded from the benchmark invocation, keeping the SpacetimeDB baseline clean and directly comparable.
- **Isolation for honest comparison** — A dedicated SpacetimeDB-only topology avoids confounding variables that would arise from running SpacetimeDB alongside Arcane infrastructure on shared hosts.

## Relationships
- [[Setup-AwsBenchmark.ps1]]
- [[Run-Benchmark-AwsRemote.ps1]]
- [[Cleanup-AwsBenchmark.ps1]]
- [[Run-Benchmark.ps1]]
- [[infra/aws/topologies/]]
- [[SpacetimeDB]]
- [[AwsArcaneCluster Topology]]
- [[Benchmark State JSON]]
- [[arcane-scaling-benchmarks]]

## Conversations That Shaped This
- [[Benchmark improvement suggestions]]