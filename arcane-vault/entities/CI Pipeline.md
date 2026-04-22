---
type: entity
tags: [ci-pipeline, github-actions, benchmarks, infrastructure, testing, automation, powershell, rust]
---

# CI Pipeline

## What It Is
The CI Pipeline for the Arcane project is a GitHub Actions–based automation system that runs across multiple repositories, primarily the `arcane` Rust workspace and the `arcane-scaling-benchmarks` harness. It enforces build correctness, runs unit and integration tests, and validates the benchmark scripting infrastructure, acting as the primary quality gate before changes reach production benchmarking or deployment.

## Origin & Evolution
The CI pipeline emerged alongside the benchmarking effort as a way to keep the rapidly evolving infrastructure reproducible. Early sessions established WSL as a viable auxiliary environment for running CI automation on non-Unreal tooling. The first significant breakage — a Pester version incompatibility where tests written for v3/v4 syntax failed under Pester 5, discovered in the March 28 session — forced a systematic audit of the test suite and surfaced the need for explicit version pinning. That same session also resolved a Node.js 20 deprecation warning from `actions/checkout@v4` and identified missing GitHub token authentication for private submodule clones as a recurring source of flakiness. The March 30 restructuring of the benchmark directory layout (`scripts/cloud/` → `infra/aws/`, `Common/` → `lib/`, etc.) required updating all CI workflow import paths and documentation references in lockstep.

## Technical Details
The pipeline targets two primary repositories: the Rust workspace (`cargo build`, `cargo test` across all crates) and `arcane-scaling-benchmarks` (Pester tests for PowerShell harness scripts, CI workflow YAML for benchmark orchestration). In GitHub Actions, jobs include standard Rust toolchain setup, Pester test execution with an explicitly pinned Pester version, and submodule checkout with authenticated token injection. The benchmark CI path delegates to PowerShell harness scripts that interact with AWS via SSM and S3; SSM execution timeouts were found to be set too short for long-running benchmark runs and were patched. The pipeline also validates that the JSON state contract between `Setup-AwsBenchmark.ps1` and `Run-Benchmark-AwsRemote.ps1` remains coherent after infrastructure script changes.

## Key Design Decisions
- **Pester version pinned explicitly** — legacy `Should` assertion syntax from Pester v3/v4 is incompatible with Pester 5; pinning prevents silent test-failure regressions
- **Private submodule authentication via GitHub token** — submodule clones for `arcane-demos` and related repos require explicit token injection in CI rather than relying on default checkout credentials
- **SSM timeout tuning for long benchmarks** — default SSM execution timeouts were too short for 30-second workload runs plus AWS overhead; patched to prevent false-failure CI runs
- **CI workflow paths updated in lockstep with directory restructuring** — when `scripts/cloud/` was renamed to `infra/aws/`, all workflow YAML import references were updated atomically to prevent broken CI from reaching `main`
- **WSL retained as CI-compatible environment** — while Windows is the primary Unreal development environment, WSL/Linux is preferred for CI pipelines and backend automation given toolchain compatibility

## Relationships
- [[arcane-scaling-benchmarks]]
- [[AWS Infrastructure]]
- [[Benchmark Harness]]
- [[Pester Test Suite]]
- [[PowerShell Scripts]]
- [[arcane-swarm]]
- [[arcane (Rust workspace)]]

## Conversations That Shaped This
- [[CI pipeline failure in Arcane Scaling Benchmarks]] (2026-03-28) — primary session; Pester fix, submodule auth, SSM timeout, feature flag patch
- [[Benchmark improvement suggestions]] (2026-03-30) — directory restructuring required CI path updates
- [[Project directory exploration and analysis]] (2026-04-12) — mapped CI pipeline position within full repository structure
- [[Unreal Engine networking library setup]] (2026-02-24) — established WSL as the CI-compatible backend automation environment