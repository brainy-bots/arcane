---
type: entity
tags: [powershell, aws, s3, benchmarks, ci-pipeline, result-collection, arcane-scaling-benchmarks]
---

# Sync-AwsBenchmarkResultsFromS3.ps1

## What It Is
`Sync-AwsBenchmarkResultsFromS3.ps1` is a PowerShell script in the `arcane-scaling-benchmarks` repository responsible for pulling benchmark result artifacts down from AWS S3 into the local workspace after a cloud benchmark run completes. It serves as the final stage of the benchmark pipeline, bridging the gap between results produced on ephemeral AWS infrastructure and the local or CI environment where they are analysed and reported.

## Origin & Evolution
The script emerged as part of the broader AWS-hosted benchmark execution pipeline built to measure Arcane's scaling characteristics at player counts that can't be tested locally. Because benchmark runs execute on remote EC2 instances and write results to S3, a dedicated retrieval step was needed to make those results available for post-processing, diffing, and archival. The script's role was mapped in detail during the April 2026 directory exploration session, which catalogued the full pipeline and identified the JSON state contract that ties provisioning, execution, and result collection together. Earlier CI debugging work (March 2026) exposed fragility in adjacent pipeline stages — SSM timeouts, subnet filter quoting bugs, missing auth for private submodule clones — providing context for why a reliable, explicit sync step matters rather than ad-hoc result retrieval.

## Technical Details
The script is part of the PowerShell harness layer of `arcane-scaling-benchmarks`, sitting alongside other scripts in the CLI-driven AWS provisioning path (as opposed to the parallel Terraform IaC path). It consumes a JSON state contract that is shared across the provisioning, execution, and collection phases, using it to resolve the correct S3 bucket and key prefix for the current benchmark run. The sync itself wraps AWS CLI S3 commands (`aws s3 sync` or equivalent) to download result files to a local output directory. It is invoked either manually via the CLI harness or as a step in the GitHub Actions CI pipeline after benchmark execution completes.

## Key Design Decisions
- **JSON state contract as the source of truth** — rather than hardcoding bucket names or run identifiers, the script reads from the shared state file, keeping result collection consistent with the provisioning and execution steps that wrote that state.
- **PowerShell 7 as the runtime** — consistent with the rest of the harness; adjacent scripts exposed PS7-specific quoting bugs (e.g. subnet filter issues in `AwsHelpers.ps1`), making version consistency important for predictable AWS CLI argument handling.
- **Explicit sync step rather than inline retrieval** — separating result download into its own script allows CI to gate post-processing on successful sync and makes the step independently re-runnable without re-executing the benchmark.

## Relationships
- [[AwsHelpers.ps1]] — shared helper module used across the PowerShell harness; known PS7 quoting bugs were patched during the March 2026 CI session
- [[arcane-scaling-benchmarks]] — parent repository containing the full benchmark pipeline
- [[GitHub Actions CI pipeline (arcane-scaling-benchmarks)]] — orchestrates this script as a post-execution step
- [[JSON state contract (benchmark pipeline)]] — shared state file that provides S3 coordinates consumed by this script
- [[Terraform path (arcane-scaling-benchmarks)]] — parallel provisioning approach that converges on the same result artifacts

## Conversations That Shaped This
- [[CI pipeline failure in Arcane Scaling Benchmarks]] — March 2026 session that debugged the full pipeline and patched adjacent infrastructure bugs, establishing the reliability context this script operates in
- [[Project directory exploration and analysis]] — April 2026 session that produced the comprehensive pipeline map and explicitly identified this script's role in the collection stage