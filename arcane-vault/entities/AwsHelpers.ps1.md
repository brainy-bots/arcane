---
type: entity
tags: [infrastructure, powershell, aws, ci-cd, benchmarks, arcane-scaling-benchmarks, scripting]
---

# AwsHelpers.ps1

## What It Is
`AwsHelpers.ps1` is a PowerShell 7 helper module in the `arcane-scaling-benchmarks` repository that provides AWS infrastructure automation for benchmark execution. It encapsulates common AWS operations — such as EC2 instance and subnet querying — used by the benchmark pipeline to provision, target, and manage cloud resources during scaling tests.

## Origin & Evolution
The file emerged as part of the benchmark infrastructure layer needed to run Arcane's scaling benchmarks on AWS. As the CI pipeline matured, a critical bug was uncovered during a March 2026 debugging session: a subnet filter quoting issue specific to PowerShell 7 caused AWS CLI calls to fail silently or return incorrect results. This was patched as part of a broader infrastructure repair effort that also addressed missing GitHub token auth for submodule clones, SSM execution timeouts, and an obsolete Cargo feature. The fix stabilized the benchmark pipeline's ability to reliably target the correct AWS resources.

## Technical Details
The script is written for **PowerShell 7** (not Windows PowerShell 5.x), and the distinction matters — the quoting rules for argument passing to external commands like the AWS CLI differ between versions. The subnet filter bug specifically involved how filter strings were passed to AWS CLI commands; PowerShell 7's argument parsing requires explicit quoting that differs from older versions. The script integrates into the broader CI pipeline, likely invoked by GitHub Actions workflow steps that provision or query AWS infrastructure before running benchmark workloads via SSM.

## Key Design Decisions
- **PowerShell 7 target** — the script is explicitly written for pwsh, not Windows PowerShell, which has different quoting semantics for external CLI invocations; this was the root cause of the subnet filter bug
- **AWS CLI wrapper approach** — rather than using AWS SDK bindings, the script wraps CLI commands, making it portable across environments where the AWS CLI is available but keeping the code in the scripting layer familiar to ops tooling
- **Centralised helpers module** — AWS calls are consolidated here rather than scattered across pipeline scripts, making the quoting fix a single-point change rather than a multi-file patch

## Relationships
- [[arcane-scaling-benchmarks]] — the repository this script lives in
- [[CI Pipeline (arcane-scaling-benchmarks)]] — the GitHub Actions pipeline that invokes this script
- [[SSM Execution Timeouts]] — a related infrastructure bug fixed in the same session
- [[Pester Tests (arcane-scaling-benchmarks)]] — the test suite that validated the CI fix in the same debugging session

## Conversations That Shaped This
- [[CI pipeline failure in Arcane Scaling Benchmarks]]