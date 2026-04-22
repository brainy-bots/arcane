---
type: conversation
date: 2026-04-12
source: cursor
tags: [arcane, benchmarks, aws, terraform, powershell, infrastructure, ci-pipeline, repository-structure]
---

# Project directory exploration and analysis

**Date:** 2026-04-12
**Source:** cursor (53 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/cursor-chat-export-pgp-demo/2026-04-12-1-Project_directory_exploration_.md`

## Summary

This conversation was a deep exploratory analysis of the `arcane-scaling-benchmarks` repository, focused on mapping out the complete directory structure, identifying all entry points, and understanding how the benchmark harness is wired together from scripts through to AWS provisioning and result collection. The goal was to build a clear mental model of the project before making architectural changes or additions.

The exploration covered four to five levels of directory depth, cataloguing the PowerShell harness scripts, Terraform infrastructure definitions, JSON benchmark configuration profiles, Pester test suite, and CI pipeline. The result was a comprehensive map showing how local and cloud benchmark workflows are separated, how they share a JSON state contract, and where the overlap and divergence between the PowerShell CLI provisioning path and the Terraform IaC path lies.

A significant finding was that two parallel AWS provisioning approaches coexist in the repository — a PowerShell-driven AWS CLI path and a Terraform path — both converging on the same benchmark execution and result collection scripts via a shared `benchmark_state` JSON contract. The CI pipeline notably runs only linting, Pester tests, and Terraform validation; it does not execute actual benchmarks, meaning all real benchmark runs are either manual or triggered via the AWS workflow scripts.

The configs directory revealed structured benchmark profiles covering cluster-size variants from 1 to 10 clusters, with configurations targeting Arcane combined with SpacetimeDB, or SpacetimeDB-only scenarios. Results are organized under a `runs/<Environment>/<RunId>/<config-stem>/` hierarchy, giving each run a clean addressable path for downstream collection and analysis.

## What Was Built

- Complete annotated map of `arcane-scaling-benchmarks` directory structure to depth 3–4
- Inventory of all PowerShell entry scripts and their roles (local, cloud setup, cloud run, cleanup, collection)
- Documented the 3-step AWS cloud benchmark workflow: `Setup-AwsBenchmark.ps1` → `Run-Benchmark-Aws.ps1` → `Cleanup-AwsBenchmark.ps1`
- Identified supporting library scripts: `BenchmarkHarnessHelpers.ps1`, `scripts/cloud/Common/*.ps1`, per-topology `environments/*/Setup.ps1|RemoteBenchmark.ps1|Cleanup.ps1`
- Documented the `benchmark_state` JSON contract that binds the PowerShell provisioning and Terraform provisioning paths to the shared execution and cleanup scripts

## Key Decisions

- **Two provisioning paths are intentionally maintained in parallel**: The PowerShell/AWS CLI path offers imperative, scriptable control for ad-hoc runs; the Terraform path offers declarative repeatability — both feed the same downstream execution scripts via the state JSON contract
- **CI does not run benchmarks**: PSScriptAnalyzer, Pester, and Terraform validation are the CI gate; real benchmark execution is kept out of CI to avoid cost and timing variability
- **Results hierarchy uses `<Environment>/<RunId>/<config-stem>/`**: This structure supports both local runs and bulk S3 sync, with `Collect-AwsBenchmarkResults.ps1` and `Sync-AwsBenchmarkResultsFromS3.ps1` targeting different granularities (all runs vs. single run)
- **Benchmark configs are JSON profiles**: Separating config from code allows the same harness to run 1-cluster through 10-cluster topologies and Arcane+SpacetimeDB vs. SpacetimeDB-only scenarios without code changes

## Problems Solved

- Resolved ambiguity about which scripts are user/CI entry points vs. internal library helpers
- Clarified the relationship between the two AWS provisioning paths (PowerShell CLI vs. Terraform) and confirmed they are complementary, not competing
- Identified the exact handoff point (state JSON) between provisioning, execution, and cleanup phases
- Mapped test coverage: 7 Pester files covering harness logic, cloud helpers, topology setup, and validation — confirming test surface before any refactoring

## Entities

- [[arcane-scaling-benchmarks]]
- [[SpaceTimeDB]]
- [[AWS Infrastructure]]
- [[CI Pipeline]]
- [[Benchmark System]]
- [[Redis]]
- [[Benchmark Journal]]

NEW entities:
- NEW: [[Benchmark State JSON]] — shared contract artifact output by both PowerShell and Terraform provisioning paths, consumed by run and cleanup scripts
- NEW: [[BenchmarkHarnessHelpers]] — PowerShell library module providing shared utilities to the benchmark harness scripts
- NEW: [[Pester Test Suite]] — 7-file test suite covering harness, cloud helpers, topology, and validation within `arcane-scaling-benchmarks`
- NEW: [[Run-Benchmark.ps1]] — primary local benchmark driver script; loads config, runs scenarios, writes results
- NEW: [[Setup-AwsBenchmark.ps1]] — step 1 of AWS workflow; provisions EC2, S3, IAM and outputs benchmark state JSON

## Related Conversations

_to be linked_