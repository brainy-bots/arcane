---
type: entity
tags: [benchmarks, infrastructure, json, state-management, powershell, terraform, aws, ci-pipeline, arcane-scaling-benchmarks]
---

# Benchmark State JSON

## What It Is
Benchmark State JSON is a shared contract file used by the `arcane-scaling-benchmarks` repository to communicate runtime state between the PowerShell harness scripts, Terraform infrastructure definitions, and CI pipeline. It acts as the single source of truth for what a benchmark run looks like at any given moment — capturing provisioning status, configuration profiles, and result collection metadata so that all parts of the system can operate on a consistent view.

## Origin & Evolution
The state file emerged as a practical solution to a coordination problem: two parallel AWS provisioning approaches (a PowerShell-driven AWS CLI path and a Terraform IaC path) needed to converge on the same benchmark execution without duplicating configuration or siloing results. Rather than having each path manage its own state, a shared JSON contract was introduced so both paths could read and write a common record of what infrastructure exists, what profile is running, and where outputs should go. This allowed the local and cloud benchmark workflows to diverge in their provisioning mechanics while remaining interoperable at the data layer.

## Technical Details
The JSON state file sits at the intersection of several subsystems within `arcane-scaling-benchmarks`. JSON benchmark configuration profiles define the parameters of a run (player counts, cluster topology, duration, etc.), while the state file tracks the live execution of those profiles — infrastructure IDs, provisioning phase, test progress, and result paths. PowerShell harness scripts read and mutate this file during the CLI-driven path; Terraform outputs are expected to reconcile with it during the IaC path. The Pester test suite likely validates state file structure and transitions as part of CI. The file format itself must be stable enough for the CI pipeline to consume without custom parsing logic.

## Key Design Decisions
- **Shared contract over per-path state** — Having two provisioning paths (PowerShell AWS CLI + Terraform) write to one file prevents drift between what was provisioned and what the harness believes is provisioned.
- **JSON as the interchange format** — JSON is readable by PowerShell, Terraform outputs, and CI runners without any bespoke serialization, keeping the contract low-friction to consume across tool boundaries.
- **Separation of configuration profiles from runtime state** — JSON benchmark configuration profiles are static inputs; the state file is a mutable runtime artifact. This split avoids overwriting tested config with ephemeral execution data.
- **Local and cloud workflow symmetry** — The state contract is the same whether a run is local or cloud-hosted, which is what allows the two workflows to share result collection and reporting logic.

## Relationships
- [[Benchmark Configuration Profiles]]
- [[PowerShell Benchmark Harness]]
- [[Terraform AWS Infrastructure]]
- [[Pester Test Suite]]
- [[CI Pipeline]]
- [[arcane-scaling-benchmarks Repository]]
- [[ClusterManager]]
- [[ClusterServer]]

## Conversations That Shaped This
- [[Project directory exploration and analysis]]