---
type: entity
tags: [benchmarks, ci, manifest, json, reproducibility, artifacts, s3, aws, pipeline]
---

# benchmark_run_manifest.json

## What It Is
`benchmark_run_manifest.json` is a structured artifact file generated at the end of each Arcane scaling benchmark run. It captures all metadata needed to uniquely identify, reproduce, and audit a benchmark execution — including configuration parameters, infrastructure details, timing, and result locations. It serves as the authoritative index for a benchmark run within the `arcane-scaling-benchmarks` pipeline.

## Origin & Evolution
The manifest emerged from the CI pipeline work in the `arcane-scaling-benchmarks` repository, where reproducibility and traceability of benchmark runs became a core concern. As the pipeline grew more complex — spanning GitHub Actions, AWS EC2/SSM execution, swarm clients, and S3 artifact storage — a single structured file was needed to tie all run outputs together. The manifest solved the problem of knowing exactly *what ran, where, under what conditions, and where the results landed*, especially important when debugging failures or comparing runs across different commits or infrastructure configurations.

## Technical Details
The manifest is written as JSON and typically uploaded to S3 alongside other benchmark artifacts (logs, metrics, result CSVs). It is generated at pipeline conclusion by the benchmark orchestration scripts (PowerShell / CI workflow steps). Key fields generally include:

- **Run identity**: run ID, timestamp, triggering commit SHA, branch
- **Configuration**: benchmark parameters (player counts, tick rates, scenario names), Arcane crate versions
- **Infrastructure**: AWS region, instance types, subnet/VPC identifiers used during the run
- **Timing**: wall-clock start/end times, per-phase durations
- **Artifact locations**: S3 paths to logs, result files, and other outputs produced by the run
- **Outcome**: pass/fail status, any error codes or anomaly flags

Because SSM execution timeouts and submodule authentication issues were root causes of past CI failures, the manifest also captures SSM execution metadata to aid post-hoc diagnosis.

## Key Design Decisions
- **JSON format** — human-readable and easily parsed by downstream tooling (CI scripts, dashboards, comparison utilities) without a special runtime dependency
- **S3 co-location with artifacts** — storing the manifest in the same S3 prefix as the run's outputs means any consumer who retrieves artifacts automatically has the manifest; no separate metadata store required
- **Immutable per-run** — the manifest is written once at run completion and not mutated; any re-run produces a new manifest with a new run ID, preserving the audit trail
- **Infrastructure fields included** — subnet IDs, instance types, and region are captured explicitly so that environment-dependent failures (e.g., the PowerShell 7 subnet filter quoting bug in `AwsHelpers.ps1`) can be correlated with specific runs
- **Commit SHA captured** — enables direct linkage between a benchmark result and the exact Arcane codebase state, critical for regression tracking across releases

## Relationships
- [[arcane-scaling-benchmarks]] — the repository that owns and produces this file
- [[AwsHelpers.ps1]] — PowerShell helper whose infrastructure outputs (subnet IDs, instance info) feed manifest fields
- [[S3 artifact storage]] — the destination where manifests are uploaded and retained
- [[GitHub Actions CI workflow]] — the pipeline stage that triggers manifest generation and upload
- [[swarm client]] — the load-generating component whose run parameters and outcomes are recorded in the manifest
- [[SSM execution]] — execution mechanism whose timeout and status metadata appear in the manifest

## Conversations That Shaped This
- [[CI pipeline failure in Arcane Scaling Benchmarks]]