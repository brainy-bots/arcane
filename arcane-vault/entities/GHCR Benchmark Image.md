---
type: entity
tags: [docker, ghcr, benchmarking, infrastructure, ci-cd, aws, rust, deployment]
---

# GHCR Benchmark Image

## What It Is
The GHCR Benchmark Image is a pre-built Docker image hosted on GitHub Container Registry (GHCR) that packages the Arcane benchmark binaries for deployment onto AWS EC2 instances. It replaces on-EC2 compilation as the mechanism for delivering benchmark executables, allowing benchmark runs to begin immediately after instance startup rather than waiting for Rust compilation to complete.

## Origin & Evolution
The image emerged from a painful bottleneck in early AWS benchmark runs: each fresh EC2 instance had to compile the Rust workspace from source before any benchmarking could begin, burning expensive instance-hours and adding significant friction to iterative testing. During the Claude Code session of 2026-04-16, the pipeline was explicitly migrated from on-EC2 compilation to pre-built Docker images pulled from GHCR — described as "dramatically reducing runtime cost and complexity." This single change unlocked a more fluid benchmark iteration loop and was a prerequisite for the production benchmark runs that established the SpacetimeDB ceiling (~250–500 players) and the Arcane ceiling (~2000 players).

## Technical Details
The image bundles the compiled benchmark binaries — including the `arcane-swarm` headless Rust client simulator and related harness crates — into a Docker container that EC2 instances pull at startup. GHCR serves as the registry, integrating naturally with the GitHub-based source repository. The benchmarking pipeline is structured so that instance initialization consists of a `docker pull` followed by container execution, rather than a full `cargo build` cycle. The image must stay in sync with the benchmark harness codebase; a stale image can produce misleading results if the wire protocol or workload parameters have changed since the last image push.

## Key Design Decisions
- **Pre-built over on-demand compilation** — Rust compile times on EC2 are substantial; moving compilation offline to CI eliminates per-run cost and wait time.
- **GHCR over ECR or DockerHub** — Co-located with the source repository on GitHub, simplifying authentication and keeping the registry in the same ecosystem as CI/CD workflows.
- **Docker as the deployment unit** — Containerisation ensures the binary runs in a reproducible environment regardless of the EC2 AMI, avoiding "works on my machine" discrepancies between benchmark runs.
- **Image must track harness changes** — Canonical workload parameters (10 Hz tick rate, 2 actions/sec, 30-second runs, spread movement) are baked into the binary; the image must be rebuilt whenever protocol or workload definitions change to keep comparisons valid.

## Relationships
- [[arcane-swarm]] — the primary binary packaged inside the image
- [[AWS EC2 Benchmark Pipeline]] — the infrastructure that pulls and runs this image
- [[Benchmark Harness]] — the broader set of crates whose compiled outputs live in the image
- [[SpacetimeDB Benchmark]] — one of the workload modes executed from the image
- [[Arcane Benchmark]] — the other workload mode executed from the image

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]] — documents the migration from on-EC2 compilation to GHCR-hosted pre-built images and the first production benchmark runs this enabled
- [[Standalone binary for Unreal Engine testing]] — provides context on the benchmark methodology and canonical workload parameters that are baked into the image binaries