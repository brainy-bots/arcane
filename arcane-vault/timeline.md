```markdown
---
type: timeline
---
# Arcane — Development Timeline

## Phase 0 — Hypothesis and Specification (2026-02-20)

Arcane began not as a library but as a research question: does clustering players by **social affinity** (guilds, parties, enemy relationships) produce better cross-cluster communication characteristics than clustering by raw spatial position? This hypothesis was the seed of everything that followed.

The first concrete work was [[Specification implementation for concept demonstration]] (2026-02-20), a session driven directly by a PDF specification. From scratch, the team built a complete PGP (Player Globe Partitioning) benchmark suite: a **Spatial Grid Server** for baseline comparison, a **PGP Cluster Manager** with HTTP coordination and spatial indexing, individual **Cluster Servers** running WebSocket simulation with TCP RPC for cross-cluster attacks, a multi-mode **Load Generator**, and a **Prometheus + Grafana** monitoring stack. The intent was to produce empirical evidence — dashboard-visible data — that affinity clustering outperforms spatial partitioning on the metrics that matter for combat-grade multiplayer.

That same day, [[Untitled Chat]] (2026-02-20) attempted to operationalize the benchmark into a continuously running visualization. It immediately surfaced data quality problems: the cluster server reported a consistent 1 kHz tick rate while the grid server flatlined at 0 Hz, most metrics stayed at zero despite low CPU load, and the system needed manual event injection to produce any meaningful output. Attempts to add real-time parameter control — adjusting player count, interactions per player, and cluster configuration mid-run — collapsed under cascading instability. The session ended with a recognition that the benchmarking methodology needed a firmer foundation before visualization could be trusted.

---

## Phase 1 — Client Architecture and Environment Setup (2026-02-24)

With the core hypothesis sketched, attention shifted to the client side. Three sessions on 2026-02-24 established the client architecture and development environment.

[[Unreal Engine setup for networking library]] (2026-02-24) began with a misalignment: early setup guidance assumed an Unreal-first plugin architecture. The user corrected this — the networking library must be **standalone**, with Unreal as one of several possible client targets. This triggered a pivot from plugin-focused toolchain configuration to a leaner MSVC Build Tools setup, establishing the architectural principle that would govern the project: Arcane replaces Unreal's native replication system rather than extending it, with Unreal serving purely as a consuming client.

[[Untitled Chat]] (2026-02-24) refined the clustering visualization and deepened the architectural model. The key insight reached here was that clustering decisions should reflect *who players are likely to interact with* — guild membership, party relationships, enemy states — not simply where they happen to stand. **Hysteresis thresholds** were introduced to prevent oscillation, where clusters would merge and immediately split again under naive threshold-based logic. The session also introduced an SDK design discussion and hit early Unreal Engine build issues.

[[Unreal Engine networking library setup]] (2026-02-24) resolved the environment question definitively: **Windows** was chosen as the primary development environment over WSL. The reasoning was clear — Unreal's editor is GPU-intensive and DirectX-dependent, the toolchain (Visual Studio, IntelliSense, debugging, plugin build pipelines) is designed Windows-first, and fighting WSL would introduce compounding friction with no compensating benefit. The `arcane-client-unreal` plugin direction was established here.

---

## Phase 2 — Core Architecture Review and Library Formation (2026-03-02 to 2026-03-03)

The largest single architectural session was [[Network library architecture review]] (2026-03-02), a 4,029-message sprint that resolved fundamental design tensions accumulated during the benchmarking and client phases. The session covered state ownership, replication topology, game logic placement, entity lifecycle, clustering cadence, and failover — producing a unified architecture coherent enough to build production software against.

The defining decision: **game logic lives in SpacetimeDB reducers, not in ClusterServers**. [[ClusterServer]] instances handle high-frequency simulation (movement, physics, AI ticks) and write owned entity state; [[SpacetimeDB]] is the single authoritative source for persistent game state and discrete game actions. This separated the concerns that had been conflated in the benchmark prototype and gave the system a clear two-tier model: fast ephemeral simulation on ClusterServers, durable authoritative state in SpacetimeDB.

[[Project documentation overview]] (2026-03-03) was a comprehensive overhaul of the demo and the library's internal structure. Two parallel tracks ran simultaneously: separating library concerns from demo-specific logic in the Rust workspace, and improving the Unreal Engine client's visual fidelity and connectivity reliability. The primary structural outcome was the creation of a dedicated `arcane-demo` crate to house all game-specific behavior (gravity, jumping, wandering, demo agents), freeing `arcane-infra` to serve as a pure clustering and replication library. A new `run_cluster_loop<F>` API was introduced to allow operator-supplied game logic to be injected into the cluster tick.

[[Untitled Chat]] (2026-03-03) produced the **four-bucket data classification model** — Spine, Replicated, Ephemeral, and Persistent — as the canonical way to reason about data lifecycle in cluster servers. This was chosen deliberately over Unreal Engine–style per-property replication flags: it reduces metadata overhead on the wire, makes replication rules explicit at the type level, and is simpler for developers to reason about. The session also produced GitHub issues capturing the cluster architecture, physics integration plan, and benchmark roadmap.

---

## Phase 3 — Replication Internals and State Distribution (2026-03-16)

[[STATE_UPDATE message handling in ClusterServer]] (2026-03-16) drilled into the replication pipeline. The investigation traced the full data path from entity state construction through to individual WebSocket client connections, clarifying how `EntityStateDelta` is built, how neighbor cluster data is merged in `cluster_runner`, and how the result is pushed over an mpsc channel to the WebSocket server.

The core finding: Arcane uses a **broadcast-first, serialize-once** pattern — the delta is serialized to JSON exactly once per tick and dropped to all connected clients without per-client filtering. This is a deliberate scalability tradeoff documented explicitly, with the analysis identifying where future per-client spatial filtering could be inserted without restructuring the pipeline.

---

## Phase 4 — Benchmark Infrastructure and AWS Deployment (2026-03-06, 2026-03-28 to 2026-03-30)

[[Standalone binary for Unreal Engine testing]] (2026-03-06) was a multi-phase engineering effort to prove Arcane's distributed cluster architecture outperforms a SpacetimeDB-only backend at scale. The session established a **fair benchmarking methodology** by building a headless Rust swarm binary (`arcane-swarm`) that simulates real game clients using the actual SpacetimeDB SDK (WebSocket + BSATN + subscriptions), replacing earlier HTTP REST polling approaches that had unfairly penalized SpacetimeDB. Canonical workload parameters were defined and the Unreal Engine demo was extended to showcase entity replication across 150–200+ concurrent networked characters.

[[Project repository status]] (2026-03-06) debugged a client-side rendering issue: mannequin characters were failing to appear in-game, traced to problematic dynamic material logic interfering with character mesh visibility. The fix stripped the dynamic material logic and produced a successful build — a short session but representative of the ongoing Unreal client stabilization work running in parallel with backend development.

[[CI pipeline failure in Arcane Scaling Benchmarks]] (2026-03-28) began as a CI failure investigation and expanded into a comprehensive debugging effort across the full benchmark pipeline. The initial breakage traced back to a **Pester version incompatibility**: tests were written for Pester v3/v4 syntax but CI installed Pester 5, causing legacy `Should` assertions to fail. This was resolved by updating test syntax. Deeper investigation uncovered additional AWS-hosted benchmark execution issues, which were also addressed during this session.

[[Benchmark improvement suggestions]] (2026-03-30) restructured the AWS benchmark scripting infrastructure around a clean separation of concerns that had been conflated in `Run-Benchmark.ps1`. The new structure: `Setup-AwsBenchmark.ps1` provisions and writes state JSON, `Run-Benchmark-AwsRemote.ps1` reads that state and delegates to the harness, `Run-Benchmark.ps1` assumes infrastructure already exists and only runs the workload, `Cleanup-AwsBenchmark.ps1` tears everything down. Multi-host Arcane parameters that had leaked into the run harness were identified for removal. The session also deepened the Arcane data model and physics architecture.

---

## Phase 5 — Repository Consolidation and Infrastructure Mapping (2026-04-12)

[[Project directory exploration and analysis]] (2026-04-12) was a deep exploratory mapping of the `arcane-scaling-benchmarks` repository — cataloguing the PowerShell harness scripts, Terraform infrastructure definitions, JSON benchmark configuration profiles, Pester test suite, and CI pipeline. The result was a comprehensive map showing how local and cloud benchmark workflows are separated, how they share a JSON state contract, and where entry points live across the directory tree. This groundwork preceded planned architectural changes.

[[Project conversation export options]] (2026-04-12) addressed tooling and archival: a bulk export of all Cursor chat history from the `pgp-demo` project using the `cursor-history` npm CLI tool. The motivation was producing a clean, browsable record rather than navigating Cursor's UI or digging through raw JSONL files. This session is the direct ancestor of the vault you are now reading.

---

## Phase 6 — Submodule Split, v0.1.0, and Benchmark Parity (2026-04-16 to 2026-04-18)

[[Claude Code session — pgp-demo]] (2026-04-16, orientation) was an orientation over the `pgp-demo` monorepo after it had been split into **five independent git submodule repositories**: `arcane` (core Rust workspace), `arcane-client-unreal` (UE5 plugin), `arcane-demos` (runnable demo project), `arcane-scaling-benchmarks` (performance testing framework), and `arcane-swarm` (headless load client). The assistant performed a comprehensive read-through of all five sub-repos to build a clear picture of the ecosystem before substantive work resumed.

[[Claude Code session — pgp-demo]] (2026-04-16, hardening and v0.1.0) was a sprawling multi-phase session. It began with merging four PRs into the core `arcane` library (entity state model, simulation trait, input validation, licensing) and **publishing v0.1.0**. Simultaneously, a fundamental flaw in the benchmark design was identified and corrected: SpacetimeDB-only mode had been running real physics while Arcane mode ran none, making comparisons meaningless. A **five-connection-type architecture** was defined and three new benchmark crates were created to produce equivalent workloads across both modes. The session also covered AWS infrastructure live runs and harness refactoring.

[[Claude Code session — pgp-demo]] (2026-04-18, environment setup) was a prerequisite tooling session: getting **PowerShell 7.6.0** installed inside WSL Ubuntu 22.04. The `apt-get install powershell` path failed because Microsoft's package is not in Ubuntu's default repositories; the fix required manually registering Microsoft's custom apt repository and signing key. No Arcane backend code was modified; this was infrastructure for running the PowerShell benchmark harness from within WSL.

[[Claude Code session — pgp-demo]] (2026-04-18, brief verification) was a short communication handshake confirming a fix had been applied, with the assistant requesting clarification on what was changed and how to verify success.

The session [[Claude Code session — e8dec835]] (date unknown) covered work in the `mnt-e-code-pgp-demo` context, likely involving iteration on the PGP demo scenario and touching `arcane-infra` (ClusterManager, ClusterServer) or spatial indexing layers. The raw transcript was not recoverable from available chunk summaries.

---

## Current State

As of the most recent sessions, Arcane is a published Rust library (v0.1.0, AGPL-3.0) organized as a five-crate workspace: `arcane-core`, `arcane-spatial`, `arcane-rules`, `arcane-pool`, and `arcane-infra`. The architecture is settled: ClusterServers own high-frequency simulation, SpacetimeDB owns persistent authoritative state, and clients connect via WebSocket. The benchmark infrastructure runs on AWS with a clean provisioning/run/teardown separation, and the comparison methodology is now fair — both Arcane and SpacetimeDB-only modes run equivalent physics workloads. The Unreal Engine client plugin (`arcane-client-unreal`) exists in a separate repo and is functional for demo purposes. The monorepo has been split into five git submodule repositories.
```