---
type: conversation
date: 2026-03-30
source: cursor
tags: [benchmark, aws-infrastructure, arcane, spacetimedb, physics, replication, state-model, scripting, powershell, architecture]
---

# Benchmark improvement suggestions

**Date:** 2026-03-30
**Source:** cursor (4908 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/cursor-chat-export-pgp-demo/2026-03-30-3-Benchmark_improvement_suggesti.md`

## Summary

This session focused on two parallel tracks: restructuring the AWS benchmark scripting infrastructure and deepening the Arcane data model and physics architecture. The scripting work began with a critical insight — provisioning, running, and cleanup had been incorrectly conflated inside `Run-Benchmark.ps1`. The session clarified a clean separation: `Setup-AwsBenchmark.ps1` provisions and writes state JSON, `Run-Benchmark-AwsRemote.ps1` reads that state and delegates to the harness, `Run-Benchmark.ps1` assumes infrastructure already exists and only runs the workload, and `Cleanup-AwsBenchmark.ps1` tears everything down. Multi-host Arcane parameters that had leaked into the run harness were identified for removal.

The directory layout was also restructured in this session. The old `scripts/cloud/` tree was reorganized into `infra/aws/`, with `Common/` renamed to `lib/`, `environments/` renamed to `topologies/`, and the `Tools/` folder removed. All import paths, CI workflow references, Pester test assertions, and documentation were updated to reflect the new layout. The result is a cleaner, more navigable project structure where AWS orchestration has a single entry point and topology-specific logic is clearly scoped.

On the architecture side, the session formalized the **four-bucket state model** for entity data in Arcane clusters: spine identity data replicated via Redis, mutable game state (position, velocity, user_data), cluster-local scratch that never touches the wire, and SpacetimeDB durable state with throttled persistence. A security bug was found and fixed — `local_data` was being deserialized from Redis JSON, violating the architecture's trust boundary — and a regression test was added. The session also established a physics backend integration plan, identifying `ClusterSimulation::on_tick` as the correct insertion point and recommending Unreal-native Chaos as v1 with an optional Rust sidecar path.

Finally, benchmark scope documentation was corrected: the benchmark runs a synthetic movement integrator, not production physics, and the docs now clearly state what is and is not being measured. Issue #6 (four-bucket model) and Issue #8 (physics backends) were updated to link concrete implementation guides, and PR #9 was merged with the four-bucket changes and security fix.

## What Was Built

- Restructured `scripts/cloud/` → `infra/aws/` with `lib/`, `topologies/` layout and all references updated
- Clean three-script separation: `Setup-AwsBenchmark.ps1`, `Run-Benchmark-AwsRemote.ps1`, `Run-Benchmark.ps1`
- `infra/Verify-BenchmarkEnvironments.ps1` as minimal orchestrator for moat + local + AWS matrix
- `EntityStateEntry` updated with `user_data`, `local_data` fields and serde guards (`skip_serializing`, `skip_deserializing` on `local_data`)
- `four-bucket-state-model.md` — canonical architecture doc with trust boundaries, merge rules, and delegation checklist
- `physics-backends-and-unreal.md` — implementer-ready guide with entity↔body mapping, JSON wire contract, phased checklist
- Updated `SYSTEM_ARCHITECTURE.md` and `CHANGELOG.md` to reflect bucket model and physics plan
- Benchmark scope documentation added clarifying synthetic vs. production physics measurement
- PR #9 merged: four-bucket changes, `skip_deserializing` security fix, regression test
- `infra/aws/README.md` and `infra/aws/topologies/README.md` updated as single entry points for AWS orchestration
- CI workflow and `tests/Layout.Tests.ps1` updated for new path layout

## Key Decisions

- **Provisioning and running are separate concerns:** `Run-Benchmark.ps1` must never provision, start, stop, or clean up AWS resources — it assumes infrastructure exists and only runs the workload
- **One topology per invocation:** no sweep of cluster counts in a single run; each invocation is one fixed topology producing one result set, selected via config file or CLI flags
- **`infra/aws/` as canonical AWS root:** cleaner than `scripts/cloud/`, with `lib/` for shared helpers and `topologies/` for per-environment setup/run/cleanup
- **Four buckets, not fine-grained replication knobs:** replication frequency tuning, AOI culling, and per-property deltas are deferred; buckets by data role/lifetime are sufficient for current benchmark and demo scope
- **`local_data` is never deserialized from Redis:** enforced via `#[serde(skip_deserializing)]` — cluster-local scratch must never be trusted from the wire
- **Physics: Unreal-native Chaos in-process as v1:** Arcane core stays physics-agnostic; separate crates/binaries per backend; optional Rust sidecar documented but not required
- **Synthetic benchmarks are valid:** a toy integrator is the correct tool for measuring replication, networking, and persistence at scale; production physics is a separate concern documented explicitly

## Problems Solved

- **Conflated provisioning and running:** `Run-Benchmark.ps1` had accumulated AWS provisioning parameters (`ArcaneManagerHost`, `ArcaneClusterHosts`, etc.) that don't belong there — identified and slated for removal
- **Stale directory layout:** `scripts/cloud/` with inconsistent naming (`Common/`, `environments/`, `Tools/`) replaced with consistent `infra/aws/lib/`, `infra/aws/topologies/` structure; all references updated across CI, tests, and docs
- **Undocumented benchmark scope:** benchmark appeared to include production physics; fixed by adding explicit scope docs without publishing known issues as defects
- **`local_data` security violation:** cluster-local scratch was being deserialized from Redis JSON — fixed with `skip_deserializing` and regression test
- **Ungrounded GitHub issues:** Issues #6 and #8 lacked links to implementation guidance; updated to point to concrete doc paths so implementers can find what they need

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
- [[Unreal Engine Client]]
- [[CI Pipeline]]
- [[Benchmark Journal]]

NEW:
- NEW: [[Four-Bucket State Model]]
- NEW: [[Physics Backends]]
- NEW: [[BenchmarkMode]]
- NEW: [[AwsSpacetimeOnly Topology]]
- NEW: [[AwsArcanePerHost Topology]]
- NEW: [[Run-Benchmark.ps1]]
- NEW: [[Setup-AwsBenchmark.ps1]]
- NEW: [[Cleanup-AwsBenchmark.ps1]]
- NEW: [[Run-Benchmark-AwsRemote.ps1]]
- NEW: [[EntityStateEntry]]
- NEW: [[ClusterSimulation]]

## Related Conversations

_to be linked_