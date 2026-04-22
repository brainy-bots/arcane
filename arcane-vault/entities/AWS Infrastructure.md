---
type: entity
tags: [aws, infrastructure, benchmarks, ec2, terraform, powershell, docker, ci-cd, s3, ssm]
---

# AWS Infrastructure

## What It Is
AWS Infrastructure in the Arcane project refers to the cloud provisioning and execution layer used to run large-scale multiplayer benchmarks that cannot be replicated locally. It encompasses EC2 instances, S3 result storage, SSM remote execution, and the scripting harness that provisions, runs, and tears down benchmark environments. It exists as a separate concern from the Arcane library itself — its sole purpose is producing credible, reproducible performance comparisons between Arcane's distributed cluster architecture and SpacetimeDB-only backends at player counts (1000–2000+) that require real cloud hardware.

## Origin & Evolution
The AWS layer emerged when benchmark workloads outgrew what could be run on a developer machine. Early benchmark sessions used HTTP REST polling against SpacetimeDB, which unfairly penalized that backend; once a fair workload was established (10 Hz tick, WebSocket + BSATN, 30-second runs), the need for scalable cloud execution became concrete. The initial implementation was PowerShell scripts driving the AWS CLI directly — provisioning EC2 instances, copying binaries, running workloads via SSM, and collecting results to S3. Several infrastructure bugs were discovered and patched during this period: a PowerShell 7 subnet filter quoting issue in `AwsHelpers.ps1`, missing GitHub token authentication for private submodule clones, and SSM execution timeouts set too short for long-running benchmark runs.

A parallel Terraform path was later introduced alongside the PowerShell CLI path, with both converging on the same JSON state contract that scripts use to hand off provisioning state to execution. A major pipeline improvement came when on-EC2 compilation was replaced with pre-built Docker images pulled from GHCR, dramatically reducing runtime cost and complexity and making benchmark runs faster and more reproducible.

## Technical Details
The infrastructure layer lives in the `arcane-scaling-benchmarks` repository under `infra/aws/` (reorganized from the original `scripts/cloud/` tree). The directory structure separates concerns into `lib/` (shared helpers, formerly `Common/`), `topologies/` (environment definitions, formerly `environments/`), and named scripts per lifecycle phase. Two provisioning paths coexist:

- **PowerShell + AWS CLI path**: `Setup-AwsBenchmark.ps1` provisions and writes a state JSON file; `Run-Benchmark-AwsRemote.ps1` reads that state and delegates to the run harness; `Run-Benchmark.ps1` assumes infrastructure exists and only executes the workload; `Cleanup-AwsBenchmark.ps1` tears everything down.
- **Terraform path**: IaC definitions for the same EC2/networking topology, converging on the same JSON state contract as the PowerShell path.

Benchmark binaries are delivered as Docker images from GHCR rather than compiled on-instance. SSM is used for remote command execution. S3 stores benchmark result artifacts. The CI pipeline (GitHub Actions) drives the full lifecycle and was updated from Node.js 16 to Node.js 20 action references; Pester tests were updated from v3/v4 syntax to Pester 5 to keep CI green.

## Key Design Decisions
- **Separation of provisioning, execution, and cleanup** — Originally conflated inside `Run-Benchmark.ps1`; split into discrete scripts so each phase can be run independently and state is handed off via JSON, enabling re-runs without re-provisioning.
- **Docker images over on-instance compilation** — Eliminates per-run compile time and cost, makes results more reproducible, and decouples the benchmark harness from the Rust toolchain on EC2.
- **Two parallel provisioning paths (PowerShell + Terraform)** — The PowerShell path was built first for speed; Terraform was added for IaC repeatability. Both share the same JSON state contract to avoid divergence at the execution layer.
- **SSM for remote execution** — Avoids the need to open SSH ports on benchmark instances, keeping the security surface minimal.
- **S3 for result persistence** — Decouples result collection from instance lifetime; instances can be terminated immediately after a run without losing data.
- **JSON state contract between scripts** — Allows CI, local developers, and the Terraform path to all hand off infrastructure state to the same run harness without duplication.

## Relationships
- [[Arcane Scaling Benchmarks]]
- [[SpacetimeDB]]
- [[ClusterManager]]
- [[Docker & GHCR Pipeline]]
- [[CI Pipeline (GitHub Actions)]]
- [[Benchmark Harness]]
- [[PowerShell Scripting Layer]]

## Conversations That Shaped This
- [[CI pipeline failure in Arcane Scaling Benchmarks]]
- [[Benchmark improvement suggestions]]
- [[Project directory exploration and analysis]]
- [[Claude Code session — pgp-demo]]
- [[Standalone binary for Unreal Engine testing]]