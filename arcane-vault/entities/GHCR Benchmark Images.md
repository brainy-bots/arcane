---
type: entity
tags: [benchmarking, docker, ghcr, ci-cd, infrastructure, rust, arcane-infra, devops]
---

# GHCR Benchmark Images

## What It Is
GHCR Benchmark Images are Docker container images published to GitHub Container Registry (ghcr.io) that package the Arcane backend binaries — primarily `arcane-cluster`, `arcane-manager`, and `arcane-swarm` — for use in reproducible, cloud-deployable benchmark runs. They serve as the distribution artifact that allows benchmark workloads to be launched on AWS and other environments without requiring a local Rust build, ensuring that performance comparisons between Arcane and SpacetimeDB are run against identical, pinned binary versions.

## Origin & Evolution
The images emerged from the need to move benchmark execution off local developer machines and onto cloud infrastructure (AWS EC2, ECS) where hardware is controlled and results are reproducible. Early benchmark iterations used ad-hoc binary builds and REST-based client simulation; as the methodology matured — locking in canonical workload parameters (10 Hz tick rate, 2 actions/sec, 30-second runs, spread movement) and replacing HTTP polling with a real WebSocket/BSATN swarm client — the need for a stable, versioned artifact became clear. Publishing to GHCR via CI pipelines allowed benchmark runs to reference a specific image tag tied to a commit or release, preventing "works on my machine" drift from contaminating performance data.

## Technical Details
Images are built from the `arcane-infra` workspace crate and typically use multi-stage Rust builds (a `cargo build --release` stage followed by a minimal runtime layer) to keep image size manageable. The registry is `ghcr.io` under the `brainy-bots` organization namespace, consistent with the project's GitHub home. Tags correspond to git tags (e.g., `v0.1.0`) or commit SHAs for pre-release benchmark snapshots. The `arcane-swarm` binary — which simulates real game clients using the SpacetimeDB SDK over WebSocket with BSATN serialization and subscriptions — is included alongside server binaries so that both the system under test and the load generator can be pulled and run from the same registry in a coordinated deployment.

## Key Design Decisions
- **GHCR over Docker Hub** — keeps images co-located with the source repository and GitHub Actions CI, simplifying authentication and provenance tracking
- **Multi-stage builds** — separates the Rust compilation environment from the runtime image, reducing final image size and attack surface
- **Versioned tags tied to git** — ensures benchmark results can always be reproduced by referencing the exact image tag used in a prior run, critical for longitudinal performance tracking
- **Swarm binary bundled separately** — `arcane-swarm` is published as its own image (or tagged variant) so load-generator and server can scale independently on different EC2 instances without bundling unnecessary binaries
- **Feature-flag-aware builds** — images are built with the appropriate Cargo feature flags (`--features manager`, `--features cluster-ws`) matching the binary's runtime role, avoiding dead code in production containers

## Relationships
- [[arcane-infra]]
- [[arcane-swarm]]
- [[arcane-cluster]]
- [[arcane-manager]]
- [[Benchmark Methodology]]
- [[AWS Benchmark Infrastructure]]
- [[SpacetimeDB Comparison Benchmarks]]
- [[CI/CD Pipeline]]

## Conversations That Shaped This
- [[Standalone binary for Unreal Engine testing]]