---
type: entity
tags: [benchmarks, infrastructure, aws, docker, arcane, spacetimedb, rust, performance]
---

# benchmark-cluster

## What It Is
The benchmark-cluster is the AWS-based infrastructure used to run controlled performance comparisons between Arcane's multiplayer backend and SpacetimeDB. It executes load tests across multiple connection types and player counts to establish the relative ceilings of each system. It serves as the empirical foundation for Arcane's scaling claims and competitive positioning.

## Origin & Evolution
The benchmark-cluster emerged from a recognized need to validate Arcane's core premise — that it can handle significantly more concurrent players than single-process alternatives like SpacetimeDB. Early benchmark design contained a critical flaw: SpacetimeDB-only mode ran real physics while Arcane mode ran none, making comparisons meaningless. This was corrected by defining a five-connection-type architecture and creating three new benchmark crates to produce equivalent workloads across both modes. Infrastructure evolved from on-EC2 compilation to pre-built Docker images pulled from GHCR, dramatically reducing runtime cost and complexity. First production runs established SpacetimeDB's ceiling at ~250–500 players and Arcane's ceiling at ~2000 players.

## Technical Details
The benchmark cluster runs on AWS EC2 and uses Docker images pre-built and hosted on GHCR (GitHub Container Registry) rather than compiled on-instance. Three benchmark crates cover distinct workload types, each exercising equivalent physics and simulation load across Arcane and SpacetimeDB modes. The five-connection-type architecture ensures apples-to-apples comparison. Results are collected from live runs and used to populate Arcane's competitive positioning documentation.

## Key Design Decisions
- **Pre-built Docker images over on-EC2 compilation** — reduces AWS runtime cost, eliminates build variability, and simplifies pipeline execution
- **Equivalent workloads across modes** — fixed the original flaw where SpacetimeDB ran real physics and Arcane did not; comparisons are now methodologically valid
- **Five-connection-type architecture** — provides coverage across the range of realistic multiplayer workload patterns, not just peak-load synthetic tests
- **AWS as the execution environment** — production-representative hardware rather than developer machines, ensuring results reflect real deployment conditions

## Relationships
- [[arcane-infra]]
- [[arcane-core]]
- [[ClusterManager]]
- [[ClusterServer]]
- [[SpacetimeDB]]
- [[arcane-scaling-benchmarks]]

## Conversations That Shaped This
- [[Claude Code session — pgp-demo]]