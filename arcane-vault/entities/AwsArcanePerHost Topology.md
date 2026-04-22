---
type: entity
tags: [aws-infrastructure, topology, benchmark, arcane, multi-host, provisioning, scaling]
---

# AwsArcanePerHost Topology

## What It Is
AwsArcanePerHost is an AWS deployment topology for the Arcane multiplayer backend in which each Arcane cluster node runs on its own dedicated EC2 host. It is one of the named topology configurations within the benchmark infrastructure, used to test and measure how Arcane scales across physically separated machines rather than in a co-located or single-host arrangement.

## Origin & Evolution
The topology emerged from the benchmark scripting restructuring work that took place around 2026-03-30. During that session, the old `scripts/cloud/` directory tree was reorganized into `infra/aws/`, and the `environments/` folder was renamed to `topologies/` — making the topology concept an explicit, first-class configuration unit. AwsArcanePerHost was one of the named topology configurations surfaced by this reorganization. The refactor also clarified that provisioning state (which hosts exist, their addresses, etc.) should be written to a state JSON file by `Setup-AwsBenchmark.ps1` and then read by the run harness, rather than being mixed into benchmark execution logic. This clean separation made per-host topologies tractable to script reliably.

## Technical Details
- Topology definitions live under `infra/aws/topologies/` following the directory restructuring.
- `Setup-AwsBenchmark.ps1` is responsible for provisioning the EC2 instances for the topology and writing a state JSON file describing the resulting infrastructure.
- `Run-Benchmark-AwsRemote.ps1` reads the state JSON and delegates to the run harness; it does not reprovision.
- `Run-Benchmark.ps1` assumes infrastructure already exists and only executes the workload — multi-host Arcane parameters that had previously leaked into this script were identified for removal.
- `Cleanup-AwsBenchmark.ps1` tears down all resources associated with the topology.
- The per-host model means each Arcane node gets its own machine, enabling isolation of per-node resource consumption and clean measurement of cross-host replication and coordination overhead.

## Key Design Decisions
- **Topology as a config unit, not a script branch** — renaming `environments/` to `topologies/` elevated the topology concept so each deployment shape (e.g., AwsArcanePerHost) is a standalone, swappable configuration rather than a conditional code path.
- **Strict separation of provisioning, execution, and teardown** — the three-script split (`Setup`, `Run`, `Cleanup`) prevents state leakage between phases and makes per-host topologies independently repeatable.
- **State JSON as the handoff artifact** — writing provisioned host details to a JSON file decouples infrastructure setup from benchmark execution, allowing the run harness to be topology-agnostic.
- **Multi-host parameters removed from the run harness** — Arcane multi-host parameters that had leaked into `Run-Benchmark.ps1` were explicitly identified for removal, keeping the run script topology-neutral.

## Relationships
- [[Setup-AwsBenchmark.ps1]]
- [[Run-Benchmark-AwsRemote.ps1]]
- [[Run-Benchmark.ps1]]
- [[Cleanup-AwsBenchmark.ps1]]
- [[infra/aws/topologies/]]
- [[Benchmark State JSON]]
- [[arcane-infra]]
- [[ClusterManager]]
- [[ClusterServer]]

## Conversations That Shaped This
- [[Benchmark improvement suggestions]]