---
type: entity
tags: [benchmarking, performance, arcane, spacetimedb, clustering, aws, ci, rust, swarm, load-testing, hypothesis-testing]
---

# Benchmark System

## What It Is
The Benchmark System is a suite of tools and infrastructure used to empirically validate Arcane's core architectural claim: that its distributed cluster architecture outperforms a SpacetimeDB-only backend at scale. It spans a Rust headless swarm client (`arcane-swarm`), AWS cloud infrastructure scripts, a CI pipeline, and monitoring via Prometheus and Grafana — together enabling reproducible, apples-to-apples comparisons between backends at 1000+ concurrent players.

## Origin & Evolution
The benchmark system began as a **PGP (Player Globe Partitioning) demo** in February 2026, built to validate whether social-affinity clustering produces better cross-cluster communication characteristics than raw spatial clustering. That first implementation exposed serious validity problems: the spatial grid server flatlined at 0 Hz, most metrics stayed at zero, and UI controls reset without effect — making the parameter space impossible to explore. A deeper structural failure was identified: despite a hardcoded 20-player-per-server limit, servers accepted unlimited players, and the "physics" was toy-grade, not authoritative.

The approach was rebuilt from the ground up in March 2026 around a headless Rust swarm binary (`arcane-swarm`) that simulates real game clients using the actual SpacetimeDB SDK (WebSocket + BSATN + subscriptions), replacing the earlier HTTP REST polling approach that had unfairly penalized SpacetimeDB. Canonical workload parameters were locked (10 Hz tick rate, 2 actions/sec, 30-second runs, spread movement, everyone-sees-everyone visibility). Key finding: SpacetimeDB-only ceiling is ~1000 concurrent players; Arcane's distributed model pushes significantly beyond that.

Further maturation in late March 2026 introduced a clean AWS scripting separation, directory restructuring (`scripts/cloud/` → `infra/aws/`), and CI pipeline hardening after a Pester version incompatibility broke GitHub Actions.

## Technical Details
- **`arcane-swarm`**: Headless Rust binary simulating real clients via SpacetimeDB SDK; used as the fair baseline client for both SpacetimeDB-only and Arcane-distributed runs.
- **Canonical workload**: 10 Hz tick rate, 2 actions/sec per player, 30-second run duration, spread movement pattern, full visibility mesh.
- **AWS scripting architecture** (post-restructure):
  - `Setup-AwsBenchmark.ps1` — provisions infrastructure, writes state JSON
  - `Run-Benchmark-AwsRemote.ps1` — reads state JSON, delegates to harness
  - `Run-Benchmark.ps1` — assumes infrastructure exists, runs workload only
  - `Cleanup-AwsBenchmark.ps1` — teardown
- **CI**: GitHub Actions pipeline; historically broken by Pester v5 incompatibility, SSM timeouts too short for long runs, missing GitHub token for private submodule clones, and an obsolete `spacetimedb-persist` cargo feature — all patched.
- **Monitoring**: Prometheus + Grafana stack for real-time metric visualization; early versions emitted tick-rate metrics at ~1 kHz instead of the intended 20 Hz (a bug fixed during the PGP phase).
- **Earlier PGP phase**: Spatial Grid Server (baseline), PGP Cluster Manager (HTTP coordination + spatial index), individual Cluster Servers (WebSocket + TCP RPC for cross-cluster combat), and a multi-mode Load Generator.

## Key Design Decisions
- **Real SDK client, not HTTP polling** — Early benchmarks used REST polling which unfairly penalized SpacetimeDB's subscription model; switching to the actual SpacetimeDB SDK WebSocket client made comparisons valid.
- **Canonical workload parameters locked early** — Prevents benchmark drift between runs and ensures results are comparable across infrastructure changes.
- **Provisioning/run/teardown separation** — Conflating these three phases inside a single script made debugging infrastructure failures nearly impossible; clean separation was a deliberate architectural correction.
- **Authoritative physics required** — Toy physics invalidated early PGP results; the benchmark only became meaningful when server-side authoritative physics (`physics_tick` scheduled reducer) was included in the SpacetimeDB baseline.
- **Headless swarm, not Unreal client** — Using a lightweight Rust binary for load generation decouples benchmark validity from engine-side complexity and allows scaling to player counts an Unreal client farm cannot reach.

## Relationships
- [[arcane-swarm]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[SpatialIndex]]
- [[SpacetimeDB Integration]]
- [[Four-Bucket Data Model]]
- [[AWS Infrastructure]]
- [[CI Pipeline]]
- [[PGP Clustering]]
- [[Prometheus & Grafana Monitoring]]

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]] (2026-02-20)
- [[Untitled Chat]] (2026-02-20) — architectural failures of PGP benchmark identified
- [[Untitled Chat]] (2026-03-03) — physics gap identified; four-bucket model introduced
- [[Standalone binary for Unreal Engine testing]] (2026-03-06) — swarm binary, canonical workload, SpacetimeDB ceiling discovered
- [[CI pipeline failure in Arcane Scaling Benchmarks]] (2026-03-28) — CI hardening, AWS infrastructure bugs patched
- [[Benchmark improvement suggestions]] (2026-03-30) — AWS script separation, directory restructure