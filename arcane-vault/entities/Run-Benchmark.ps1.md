---
type: entity
tags: [powershell, benchmark, harness, scripting, aws, infrastructure, ci-pipeline, arcane-scaling-benchmarks]
---

# Run-Benchmark.ps1

## What It Is
`Run-Benchmark.ps1` is the core benchmark execution harness in the `arcane-scaling-benchmarks` repository. It assumes infrastructure is already provisioned and is solely responsible for running the benchmark workload — it does not provision, configure, or tear down any AWS resources. It is the runtime heart of a larger multi-script pipeline designed to measure and compare Arcane and SpacetimeDB at scale.

## Origin & Evolution
The script emerged from a critical architectural insight reached in the 2026-03-30 session: provisioning, running, and cleanup had been incorrectly conflated inside a single script. The refactor drew a clean boundary between four responsibilities — provisioning (`Setup-AwsBenchmark.ps1`), execution (`Run-Benchmark.ps1`), remote delegation (`Run-Benchmark-AwsRemote.ps1`), and teardown (`Cleanup-AwsBenchmark.ps1`). Multi-host Arcane parameters that had leaked into the run harness were identified and removed as part of this separation. Later, the broader project directory was restructured (renaming `scripts/cloud/` to `infra/aws/`, `Common/` to `lib/`, etc.), and all import paths referencing this script were updated accordingly.

## Technical Details
`Run-Benchmark.ps1` operates against a pre-existing infrastructure state described by a JSON state file written by `Setup-AwsBenchmark.ps1`. It reads connection parameters and topology from that contract rather than accepting raw AWS provisioning arguments. The script sits in a pipeline where `Run-Benchmark-AwsRemote.ps1` acts as the cloud-facing wrapper — it reads state JSON and delegates execution to `Run-Benchmark.ps1`, while `Run-Benchmark.ps1` itself focuses purely on workload orchestration. In the later Docker-based pipeline, it coordinated benchmark runs against pre-built images pulled from GHCR rather than compiling on EC2, reducing runtime cost and complexity.

## Key Design Decisions
- **Single-responsibility scoping** — `Run-Benchmark.ps1` does one thing: run the workload. Provisioning and teardown are explicitly out of scope, preventing state management bugs that arise when setup and execution are entangled.
- **JSON state contract** — infrastructure details flow in via a state file written by the provisioning script, not via CLI flags repeated across scripts; this creates a stable interface between pipeline stages.
- **No multi-host Arcane parameters** — Arcane-specific cluster topology parameters were removed from this script during the 2026-03-30 refactor; they belong to provisioning, not execution.
- **Docker image pipeline** — the script adapted to pull pre-built GHCR images rather than triggering on-EC2 compilation, a shift that dramatically reduced benchmark run time and infrastructure cost.

## Relationships
- [[Setup-AwsBenchmark.ps1]]
- [[Run-Benchmark-AwsRemote.ps1]]
- [[Cleanup-AwsBenchmark.ps1]]
- [[arcane-scaling-benchmarks]]
- [[SpacetimeDB Benchmark]]
- [[Arcane Benchmark]]
- [[AWS Benchmark Infrastructure]]
- [[JSON State Contract]]

## Conversations That Shaped This
- [[Benchmark improvement suggestions]]
- [[Project directory exploration and analysis]]
- [[Claude Code session — pgp-demo]]