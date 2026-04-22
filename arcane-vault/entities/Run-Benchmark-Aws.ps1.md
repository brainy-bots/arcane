---
type: entity
tags: [powershell, benchmarks, aws, ci-pipeline, infrastructure, orchestration, arcane-scaling-benchmarks]
---

# Run-Benchmark-Aws.ps1

## What It Is
`Run-Benchmark-Aws.ps1` is the primary orchestration script for executing Arcane scaling benchmarks against AWS infrastructure. It serves as the top-level entry point for cloud benchmark runs, coordinating AWS resource provisioning, benchmark execution via SSM, and result collection into a shared JSON state contract consumed by the rest of the harness.

## Origin & Evolution
The script emerged as the cloud counterpart to local benchmark execution, providing a repeatable path for running Arcane's scaling tests at realistic infrastructure scale. It was part of a design where local and cloud benchmark workflows are kept deliberately separate but share a common JSON state contract — allowing result collection, comparison, and CI reporting to be engine-agnostic with respect to where the benchmarks ran.

During the 2026-03-28 CI investigation, several infrastructure bugs surfaced in the AWS execution path this script depends on: PowerShell 7 subnet filter quoting issues in `AwsHelpers.ps1`, missing GitHub token authentication for private submodule clones during instance bootstrap, SSM execution timeouts set too short for long-running benchmark workloads, and an obsolete `spacetimedb-persist` cargo feature breaking remote builds. These were patched as part of restoring end-to-end AWS benchmark reliability. The 2026-04-12 exploration confirmed that `Run-Benchmark-Aws.ps1` sits at the top of the PowerShell-driven AWS CLI provisioning path — one of two parallel provisioning approaches in the repository, the other being Terraform.

## Technical Details
The script drives the PowerShell-native AWS provisioning path (as opposed to the Terraform IaC path), using AWS CLI calls mediated through `AwsHelpers.ps1` for resource lookup and instance management. Benchmark execution on remote instances is dispatched via AWS SSM Run Command, with results collected back to local state. The JSON state contract it produces (or appends to) is the shared interface between the cloud execution path and downstream reporting/comparison tooling. The script operates within the broader `arcane-scaling-benchmarks` harness, which also includes Pester-based tests, JSON benchmark configuration profiles, and GitHub Actions CI integration.

## Key Design Decisions
- **PowerShell-native AWS CLI path** — chosen alongside (not replacing) Terraform, creating two parallel provisioning approaches that converge on the same JSON state contract; gives operators a scriptable, lower-overhead path without full IaC ceremony
- **SSM for remote execution** — avoids SSH key management on benchmark instances; introduced a timeout sensitivity problem for long-running benchmarks, addressed by increasing SSM execution timeout thresholds
- **Shared JSON state contract** — decouples where benchmarks run (local vs. AWS) from how results are consumed; local and cloud workflows remain independently executable while feeding the same downstream tooling
- **Dependency on `AwsHelpers.ps1`** — subnet filter queries and instance lookup are centralized in a helper module; a PowerShell 7 quoting regression in that helper required a targeted patch without changes to the orchestration script itself

## Relationships
- [[AwsHelpers.ps1]]
- [[arcane-scaling-benchmarks]]
- [[CI Pipeline (GitHub Actions)]]
- [[Terraform AWS Provisioning]]
- [[Pester Test Suite]]
- [[SSM Run Command Integration]]
- [[JSON State Contract]]
- [[SwarmClient]]

## Conversations That Shaped This
- [[CI pipeline failure in Arcane Scaling Benchmarks]]
- [[Project directory exploration and analysis]]