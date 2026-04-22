---
type: entity
tags: [powershell, aws, infrastructure, benchmarks, provisioning, scripting, arcane-scaling-benchmarks]
---

# Setup-AwsBenchmark.ps1

## What It Is
`Setup-AwsBenchmark.ps1` is the provisioning entry point for the Arcane scaling benchmark's AWS infrastructure. It creates the cloud resources needed to run a distributed benchmark and writes the resulting state (instance IDs, IPs, etc.) to a JSON file that downstream scripts consume. It is the first stage in a four-script lifecycle: setup → run-remote → run → cleanup.

## Origin & Evolution
The script emerged from a critical architectural insight in the March 2026 session: the original `Run-Benchmark.ps1` had incorrectly conflated provisioning, workload execution, and teardown into a single script. Separating these concerns produced the clean four-script contract. `Setup-AwsBenchmark.ps1` was carved out to own provisioning exclusively, making it idempotent and safe to call independently before any workload runs. The same session also restructured the directory layout, moving the script from the old `scripts/cloud/` tree into `infra/aws/` as part of a broader renaming (`Common/` → `lib/`, `environments/` → `topologies/`).

## Technical Details
The script drives AWS resource creation via the AWS CLI (a PowerShell-native CLI path, distinct from the parallel Terraform path that also exists in the repo). On completion it serialises infrastructure state — instance IDs, public IPs, topology metadata — to a JSON file. That JSON file is the contract shared with `Run-Benchmark-AwsRemote.ps1`, which reads it to locate hosts and delegate to the benchmark harness. Multi-host Arcane parameters that had historically leaked into the run harness were identified in this session as belonging to the setup stage and were moved accordingly. Import paths within the script reference the reorganised `lib/` directory for shared helpers.

## Key Design Decisions
- **Strict separation of provisioning from execution** — rationale: allows infrastructure to be created once and benchmarks re-run without reprovisioning, and makes teardown safe to run independently.
- **JSON state file as the inter-script contract** — rationale: decouples setup from the run harness; any script in the pipeline can be replaced or tested in isolation by supplying a valid state file.
- **AWS CLI path kept alongside Terraform** — rationale: both approaches were already present in the repo; the CLI path gives a lightweight provisioning option without requiring Terraform state management.
- **Moved into `infra/aws/`** — rationale: the old `scripts/cloud/` layout mixed concerns; the new tree makes the infrastructure boundary explicit and aligns with the `topologies/` rename.

## Relationships
- [[Run-Benchmark-AwsRemote.ps1]] — reads the JSON state file written by this script
- [[Run-Benchmark.ps1]] — downstream workload runner; assumes infrastructure already exists
- [[Cleanup-AwsBenchmark.ps1]] — tears down what this script creates
- [[arcane-scaling-benchmarks]] — the repository this script lives in
- [[Terraform AWS path]] — parallel provisioning approach converging on the same JSON state contract
- [[lib/ (Common)]] — shared PowerShell helpers imported by this script

## Conversations That Shaped This
- [[Benchmark improvement suggestions]] — session where the four-script separation was defined and the directory restructure was executed
- [[Project directory exploration and analysis]] — session that mapped the full repo structure and identified the two parallel provisioning paths