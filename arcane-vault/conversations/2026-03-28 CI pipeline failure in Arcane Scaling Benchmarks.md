---
type: conversation
date: 2026-03-28
source: cursor
tags: [ci-pipeline, benchmarks, aws, swarm-client, bug-fix, spacetimedb, arcane, rust, powershell, s3, reproducibility]
---

# CI pipeline failure in Arcane Scaling Benchmarks

**Date:** 2026-03-28
**Source:** cursor (2115 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/cursor-chat-export-pgp-demo/2026-03-28-6-CI_pipeline_failure_in_Arcane_.md`

## Summary

This session began as a CI failure investigation in the `arcane-scaling-benchmarks` repository and expanded into a comprehensive debugging and optimization effort spanning the full benchmark pipeline — from GitHub Actions CI through AWS infrastructure to the swarm client code itself.

The initial CI breakage traced back to a Pester version incompatibility: tests were written for Pester v3/v4 syntax but CI installed Pester 5, causing legacy `Should` assertions to fail. This was resolved by updating test syntax, bringing CI back to green. A Node.js 20 deprecation warning from `actions/checkout@v4` was identified as an internal GitHub Actions concern unrelated to the application stack.

A deeper investigation into AWS-hosted benchmark execution uncovered several infrastructure bugs: a PowerShell 7 subnet filter quoting issue in `AwsHelpers.ps1`, missing GitHub token authentication for private submodule clones, SSM execution timeouts set too short for long-running benchmarks, and an obsolete `spacetimedb-persist` cargo feature. Each was patched systematically. Results retrieval was also formalized — `Run-Benchmark-Aws.ps1` now syncs S3 results back to a local directory automatically, and a standalone `Sync-AwsBenchmarkResultsFromS3.ps1` utility was added for manual retrieval.

The most significant discovery was a structural bug in the swarm client that had been artificially suppressing benchmark ceilings — SpacetimeDB appeared to plateau at ~250 players rather than the expected 1000+. The root cause was an entity ID mismatch: the movement loop created players with random UUIDs via `Player::new()`, while the action loop used pre-allocated IDs from `all_ids[idx]`. This meant actions hit wrong or nonexistent entities, creating orphaned rows and inflating DB load. The same bug affected Arcane benchmark runs. Once identified, the fix involved unifying entity ID allocation across all swarm client loops. A `benchmark_run_manifest.json` schema (version 3) was also introduced per run to capture full reproducibility metadata including binary SHA-256, git HEADs, CLI flags, and host metadata — written in a `finally` block to ensure capture even on failure.

## What Was Built

- Patched `AwsHelpers.ps1` with corrected PowerShell 7 subnet filter quoting
- Updated `Run-Benchmark-Aws.ps1` with S3→local result sync, GitHub token auth for private submodules, and correct SSM `executionTimeout` (28800 seconds)
- New `Sync-AwsBenchmarkResultsFromS3.ps1` standalone utility for manual result retrieval
- `benchmark_run_manifest.json` per-run schema (version 3) capturing full reproducibility metadata
- Swarm client entity ID unification fix (movement loop and action loop now share the same pre-allocated IDs)
- Updated CI test syntax for Pester 5 compatibility
- Removed obsolete `spacetimedb-persist` cargo feature from build

## Key Decisions

- **Manifest written in `finally` block:** Ensures reproducibility metadata is captured even when benchmark runs fail, maximizing diagnostic value of partial runs
- **S3 sync as default post-run behavior:** Rather than leaving results only in S3, the AWS benchmark script now pulls them locally automatically, reducing operator friction and risk of losing results
- **SSM timeout set to 8 hours (28800s):** Conservative upper bound chosen to accommodate the longest expected benchmark suites without manual intervention
- **Unified entity ID allocation in swarm client:** Pre-allocated `all_ids` array now used across all loops (movement, actions) to eliminate the identity mismatch that was corrupting load distribution
- **Pester 5 as canonical test target:** CI now assumed to always install current Pester 5; legacy syntax removed rather than pinning old version

## Problems Solved

- Pester v3/v4 `Should` syntax incompatibility causing CI test failures under Pester 5
- PowerShell 7 subnet filter string splitting on commas in `AwsHelpers.ps1`
- Private submodule clone failures on EC2 due to missing GitHub token
- SSM execution timeout too short (1-hour default) for 2+ hour benchmark runs
- `executionTimeout` parameter validation error (seconds vs milliseconds)
- Results stranded in S3 with no automated local retrieval
- Obsolete `spacetimedb-persist` cargo feature breaking builds
- Critical swarm client bug: entity ID mismatch between movement and action loops artificially capping SpacetimeDB and Arcane benchmark ceilings at ~250 players instead of expected 1000+

## Entities

- [[arcane-scaling-benchmarks]]
- [[SpaceTimeDB]]
- [[arcane_swarm]]
- [[AWS Infrastructure]]
- [[CI Pipeline]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[Benchmark System]]
- [[Benchmark Journal]]

NEW:
- NEW: [[Pester 5]] — PowerShell testing framework; version incompatibility was the root cause of initial CI failures
- NEW: [[benchmark_run_manifest.json]] — Per-run reproducibility artifact capturing schema v3 metadata: binary SHA-256, git HEADs, CLI flags, host metadata, harness timing
- NEW: [[Sync-AwsBenchmarkResultsFromS3.ps1]] — Standalone utility script for manually retrieving benchmark results from S3 to local filesystem
- NEW: [[AwsHelpers.ps1]] — AWS infrastructure helper script; patched for PowerShell 7 subnet filter quoting bug
- NEW: [[Run-Benchmark-Aws.ps1]] — Primary AWS benchmark execution script; extended with S3 sync, GitHub token auth, and corrected SSM timeout

## Related Conversations

_to be linked_