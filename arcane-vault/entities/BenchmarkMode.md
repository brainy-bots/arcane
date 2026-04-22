---
type: entity
tags: [benchmark, testing, simulation, arcane, spacetimedb, physics, aws, infrastructure, harness]
---

# BenchmarkMode

## What It Is
BenchmarkMode is the operational configuration that determines which backend system a benchmark run targets and what kind of workload it executes. In the Arcane project, BenchmarkMode distinguishes between running against SpacetimeDB-only, Arcane-only, or hybrid configurations — ensuring that comparisons between backends are valid and equivalent. It is a foundational concept in the `arcane-scaling-benchmarks` project that governs how the harness sets up connections, dispatches load, and collects results.

## Origin & Evolution
BenchmarkMode emerged from a critical design flaw discovered during early benchmark work: the original setup had SpacetimeDB-only mode running real physics while Arcane mode ran none, making any performance comparison meaningless. This asymmetry was identified and corrected in the Claude Code session of 2026-04-16, which defined a five-connection-type architecture and created new benchmark crates to produce equivalent workloads across both systems. Prior to this, the concept of mode was implicit and inconsistently applied; after the fix it became an explicit, first-class parameter governing what each benchmark run actually measures.

The scripting infrastructure around BenchmarkMode was further refined in the 2026-03-30 session, which clarified the separation between provisioning (`Setup-AwsBenchmark.ps1`), execution (`Run-Benchmark.ps1`/`Run-Benchmark-AwsRemote.ps1`), and cleanup (`Cleanup-AwsBenchmark.ps1`). Multi-host Arcane parameters that had leaked into the run harness were identified and removed, tightening the boundary between infrastructure state and mode-specific workload configuration.

## Technical Details
BenchmarkMode drives three key concerns in the harness:

1. **Connection topology** — determines which servers are targeted (SpacetimeDB module, Arcane ClusterManager HTTP join endpoint, Arcane ClusterServer WebSocket, or some combination) and how many connections of each type are established.
2. **Workload equivalence** — each mode must exercise a comparable physics/simulation path so that throughput and latency numbers reflect backend capability rather than benchmark asymmetry. The five-connection-type architecture introduced in April 2026 encodes this equivalence contract.
3. **Infrastructure parameterization** — the AWS pipeline reads mode from state JSON written by `Setup-AwsBenchmark.ps1` and passes it to the harness at runtime. Docker images pre-built for GHCR replaced on-EC2 compilation, so the mode selection happens at container invocation rather than build time.

Benchmark crates in the workspace are organized around mode variants, with separate crates producing equivalent load for SpacetimeDB-only and Arcane targets. Established performance ceilings: SpacetimeDB-only ~250–500 players, Arcane ~2000 players.

## Key Design Decisions
- **Explicit mode parameter over implicit defaults** — rationale: the original implicit approach caused the SpacetimeDB/Arcane asymmetry bug; making mode explicit forces the harness author to declare what they are testing.
- **Equivalent physics paths per mode** — rationale: benchmarks that don't exercise the same code paths measure infrastructure overhead rather than capability; the five-connection-type architecture enforces parity.
- **Mode read from provisioning state JSON, not hardcoded in scripts** — rationale: decoupling provisioning from execution allows the same infrastructure to be reused across mode variants without re-provisioning.
- **Separate benchmark crates per mode** — rationale: isolates workload logic, prevents accidental coupling between SpacetimeDB and Arcane load generation paths, and makes CI-level test targeting straightforward.

## Relationships
- [[ClusterManager]] — Arcane mode connects through ClusterManager's HTTP join endpoint
- [[ClusterServer]] — Arcane mode establishes WebSocket connections to ClusterServer
- [[SpacetimeDB]] — SpacetimeDB-only mode targets SpacetimeDB modules directly
- [[BenchmarkHarness]] — the harness reads BenchmarkMode and dispatches accordingly
- [[AWSBenchmarkPipeline]] — infrastructure provisioning writes mode into state JSON consumed at run time
- [[SimulationTrait]] — both modes must exercise equivalent simulation paths for valid comparison
- [[arcane-infra]] — provides the Arcane-side binaries (`arcane-cluster`, `arcane-manager`) targeted by Arcane benchmark modes

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]] — identified the physics-asymmetry flaw, defined five-connection-type architecture, created mode-specific benchmark crates, established player ceiling numbers
- [[Benchmark improvement suggestions]] — clarified script separation (setup/run/cleanup), removed multi-host Arcane parameters from run harness, restructured directory layout for the benchmark infrastructure