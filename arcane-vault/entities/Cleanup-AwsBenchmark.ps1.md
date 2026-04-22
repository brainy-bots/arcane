---
type: entity
tags: [powershell, aws-infrastructure, benchmarks, scripting, cleanup, teardown, arcane-scaling-benchmarks]
---

# Cleanup-AwsBenchmark.ps1

## What It Is
`Cleanup-AwsBenchmark.ps1` is the teardown script in the Arcane scaling benchmark infrastructure, responsible exclusively for destroying AWS resources provisioned for a benchmark run. It is one of four clearly separated scripts in the benchmark lifecycle, handling only the cleanup phase and nothing else. Its focused scope prevents accidental conflation of teardown logic with provisioning or workload execution.

## Origin & Evolution
The script emerged from a critical architectural insight in the 2026-03-30 session: the original `Run-Benchmark.ps1` had incorrectly conflated provisioning, running, and cleanup into a single script. This created entanglement that made the benchmark lifecycle fragile and hard to reason about. The session imposed a clean four-way separation of concerns — provision, run-remote, run-workload, and teardown — with `Cleanup-AwsBenchmark.ps1` taking sole ownership of the last phase. As part of the same restructuring, the directory layout was reorganized from `scripts/cloud/` to `infra/aws/`, and all import paths, CI workflow references, and Pester test assertions were updated accordingly.

## Technical Details
The script reads the JSON state file written by [[Setup-AwsBenchmark.ps1]] to identify which AWS resources were provisioned, then tears them down via the AWS CLI. It shares library code from `infra/aws/lib/` (formerly `scripts/cloud/Common/`) with the other benchmark scripts. It operates under the assumption that infrastructure already exists — it does not attempt to provision or run anything. The JSON state contract is the handoff boundary: `Setup-AwsBenchmark.ps1` writes it, `Run-Benchmark-AwsRemote.ps1` reads it to delegate to the harness, and `Cleanup-AwsBenchmark.ps1` reads it to know what to destroy. This design exists in parallel with a Terraform-based provisioning path, both converging on the same JSON state contract.

## Key Design Decisions
- **Single responsibility for teardown** — rationale: prevents the failure mode where cleanup logic is skipped or entangled inside a run script, ensuring resources are always destroyable independently
- **State-driven teardown via JSON contract** — rationale: decouples cleanup from provisioning logic; the script only needs to know what was created, not how it was created
- **No provisioning or workload logic** — rationale: the 2026-03-30 refactor explicitly removed multi-host Arcane parameters and run-harness logic from scripts that shouldn't carry them, keeping each script's blast radius minimal
- **Shared `lib/` imports** — rationale: common utilities (credential helpers, logging, AWS wrappers) live in one place to avoid drift across the four lifecycle scripts

## Relationships
- [[Setup-AwsBenchmark.ps1]] — writes the JSON state that this script reads to identify resources to destroy
- [[Run-Benchmark-AwsRemote.ps1]] — sibling script that reads the same state to delegate benchmark execution
- [[Run-Benchmark.ps1]] — sibling script that runs the workload assuming infrastructure exists; the original script from which cleanup was extracted
- [[arcane-scaling-benchmarks]] — the repository this script lives in

## Conversations That Shaped This
- [[Benchmark improvement suggestions]] — origin session where the four-way script separation was defined and cleanup was extracted as a standalone responsibility
- [[Project directory exploration and analysis]] — confirmed the script's place in the full directory map and its relationship to the parallel Terraform provisioning path