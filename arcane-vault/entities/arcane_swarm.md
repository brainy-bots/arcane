---
type: entity
tags: [arcane, benchmarks, load-testing, rust, swarm, spacetimedb, headless-client, performance, simulation]
---

# arcane_swarm

## What It Is
`arcane-swarm` is a headless Rust binary that simulates real game clients at scale for benchmarking the Arcane multiplayer backend. It drives load against both SpacetimeDB and Arcane cluster targets using the actual wire protocols (WebSocket + BSATN + subscriptions), producing apples-to-apples throughput and latency measurements. It lives in its own repository as a submodule of the `pgp-demo` monorepo.

## Origin & Evolution
The swarm emerged from a fundamental methodological problem: early benchmarks used HTTP REST polling to compare SpacetimeDB performance against Arcane, which unfairly penalized SpacetimeDB by bypassing its native subscription model. The fix was to build a purpose-built load generator that uses the actual SpacetimeDB SDK exactly as a real client would. This was established in the March 2026 benchmarking session alongside canonical workload parameters — 10 Hz tick rate, 2 actions/second, 30-second runs, spread movement, everyone-sees-everyone visibility — that became the locked-down standard for all future comparisons. The swarm later became one of several benchmark crates in a five-connection-type architecture designed to ensure both SpacetimeDB-only and Arcane modes receive equivalent workloads (resolving a second flaw where SpacetimeDB ran real physics but Arcane did not).

## Technical Details
- **Binary name:** `arcane-swarm`, compiled as a headless Rust crate
- **Protocol:** Uses the SpacetimeDB Rust SDK (WebSocket transport, BSATN serialization, subscription-based state delivery) — not REST polling
- **Deployment:** Runs on AWS EC2 as part of the benchmark pipeline; later migrated to pre-built Docker images pulled from GHCR to eliminate on-instance compilation overhead
- **Canonical workload parameters:**
  - Tick rate: 10 Hz
  - Action rate: 2 actions/second per client
  - Run duration: 30 seconds
  - Movement pattern: spread (not clustered)
  - Visibility: everyone-sees-everyone
- **Integration:** Orchestrated by PowerShell benchmark harness scripts (`Run-Benchmark.ps1`, `Setup-AwsBenchmark.ps1`); results fed into S3 for analysis
- **CI role:** Part of the GitHub Actions pipeline in `arcane-scaling-benchmarks`; CI breakages (Pester version mismatch, SSM timeout misconfiguration, missing submodule auth) were debugged and resolved in dedicated sessions

## Key Design Decisions
- **Use real SDK, not REST polling** — Early HTTP-based tests penalized SpacetimeDB; switching to the native WebSocket+BSATN client makes the comparison fair and representative of actual game client behavior
- **Locked canonical parameters** — Fixing workload constants (tick rate, action rate, duration, movement, visibility) across all runs ensures reproducibility and prevents benchmark drift between sessions
- **Headless standalone binary** — Separating load generation from the backend under test allows swarm instances to be scaled independently on AWS without coupling to server provisioning
- **Docker-image deployment** — Migrating from on-EC2 Rust compilation to pre-built GHCR images reduced benchmark runtime cost and eliminated environment-variance bugs
- **Five-connection-type architecture** — The swarm participates in a broader harness that tests SpacetimeDB-only, Arcane-only, and hybrid modes with equivalent physics workloads, preventing the earlier asymmetry where only one mode ran server-side physics

## Relationships
- [[arcane]] — the backend system under test
- [[arcane-scaling-benchmarks]] — the repository that orchestrates swarm runs and collects results
- [[arcane-infra]] — the Arcane cluster binary the swarm connects to in Arcane-mode benchmarks
- [[spacetimedb]] — the alternative backend the swarm benchmarks against
- [[benchmark_harness]] — the PowerShell pipeline that provisions infrastructure, invokes the swarm, and tears down resources

## Conversations That Shaped This
- [[Standalone binary for Unreal Engine testing]] (2026-03-06) — origin of the swarm concept and canonical workload parameters; SpacetimeDB ceiling established at ~1000 players
- [[CI pipeline failure in Arcane Scaling Benchmarks]] (2026-03-28) — swarm CI debugging: Pester v5 incompatibility, SSM timeouts, submodule auth, obsolete cargo features
- [[Benchmark improvement suggestions]] (2026-03-30) — harness restructuring and five-connection-type architecture context
- [[Claude Code session — pgp-demo]] (2026-04-16, `ab0c3fbc`) — Docker migration, first production runs, Arcane ceiling established at ~2000 players