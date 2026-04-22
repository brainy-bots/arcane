---
type: entity
tags: [testing, ci-pipeline, powershell, pester, compatibility, benchmarks]
---

# Pester 5

## What It Is
Pester 5 is the current major version of the PowerShell testing framework used in the `arcane-scaling-benchmarks` CI pipeline. It introduced breaking changes to test assertion syntax compared to Pester v3/v4, making it a compatibility concern for any existing test suites written against older Pester conventions. In the Arcane project, it is the version automatically installed by CI environments (GitHub Actions), which caused a pipeline breakage when legacy tests were run against it.

## Origin & Evolution
The incompatibility surfaced during a CI failure investigation on 2026-03-28 in the `arcane-scaling-benchmarks` repository. Tests had been written using Pester v3/v4 `Should` assertion syntax, but the CI runner installed Pester 5 by default. This caused the legacy assertions to fail, breaking the pipeline. The fix was straightforward: updating the test syntax to conform to Pester 5's API, which restored CI to green. The episode was one of several infrastructure bugs uncovered in the same session, alongside AWS subnet filter quoting issues, missing GitHub token auth, SSM timeout misconfigurations, and an obsolete cargo feature.

## Technical Details
Pester 5 changed the way `Should` assertions are written and evaluated compared to earlier versions. The exact nature of the breaking change involves updated parameter syntax and stricter assertion pipelines. In the `arcane-scaling-benchmarks` context, the affected tests were PowerShell scripts validating CI and infrastructure behavior — not the Rust library itself. The resolution required editing test files to use Pester 5-compatible assertion patterns rather than pinning to an older Pester version, meaning the project now targets Pester 5 as its baseline.

## Key Design Decisions
- **Upgrade tests to Pester 5 rather than pin to older version** — keeps the project aligned with what CI installs by default, avoiding version-pinning maintenance burden
- **Treat Pester tests as infrastructure validation, not Rust unit tests** — the Pester suite covers PowerShell/AWS helper scripts; Rust correctness is handled by `cargo test`

## Relationships
- [[arcane-scaling-benchmarks]] — the repository where the Pester 5 incompatibility was discovered and fixed
- [[GitHub Actions CI]] — the CI environment that installs Pester 5 by default, triggering the breakage
- [[AwsHelpers.ps1]] — a PowerShell helper script in the same repo, subject to the same CI pipeline
- [[SSM Execution]] — another infrastructure concern fixed in the same debugging session

## Conversations That Shaped This
- [[CI pipeline failure in Arcane Scaling Benchmarks]]