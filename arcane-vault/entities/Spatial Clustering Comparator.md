---
type: entity
tags: [benchmarking, clustering, spatial-grid, pgp, hypothesis-testing, rust, arcane-spatial, rules-engine]
---

# Spatial Clustering Comparator

## What It Is
The Spatial Clustering Comparator is a benchmark harness built to empirically test the core architectural hypothesis underlying the Arcane platform: whether clustering players by **social affinity** (guilds, parties — PGP: Player Globe Partitioning) produces better cross-cluster communication characteristics than clustering by raw **spatial position**. It serves as the empirical backbone for validating one of Arcane's foundational design choices before committing it to production infrastructure.

## Origin & Evolution
The comparator emerged from a dedicated benchmarking session (2026-02-20) driven by a PDF specification for the PGP demo. The problem it addresses is fundamental: Arcane's clustering strategy diverges from the naive "put nearby players on the same server" approach, but that claim needed empirical support. The benchmark was constructed in phases — first a **Spatial Grid Server** as a baseline, then a **PGP Cluster Manager** with HTTP coordination and spatial indexing, then individual **Cluster Servers** running WebSocket simulation with TCP RPC for cross-cluster attacks. A multi-mode Load Generator and a Prometheus + Grafana monitoring stack were added to capture metrics. Numerous bugs were fixed during construction: parameter ordering in the spatial index, incorrect RPC failure reporting, Docker networking for service discovery, and a tick rate metric emitting at ~1 kHz instead of the intended 20 Hz.

## Technical Details
The comparator consists of two parallel cluster configurations run against the same load scenarios:

- **Spatial Grid Server** — baseline configuration partitioning players by 2D grid position, backed by `arcane-spatial`'s `SpatialIndex`
- **PGP Cluster Manager** — experimental configuration partitioning players by social affinity (guild/party membership), with HTTP coordination between a manager process and individual cluster servers

Each cluster server runs WebSocket simulation for client connections and TCP RPC for cross-cluster attack/event propagation. A multi-mode Load Generator drives both configurations under identical conditions. Metrics are scraped by Prometheus and visualised in Grafana, with a web trigger panel for test orchestration. The key measurement targets are cross-cluster communication volume, RPC failure rates, and tick rate stability under load.

## Key Design Decisions
- **Baseline-first construction** — the spatial grid server was built before PGP so comparisons have a controlled reference point rather than comparing PGP against theory
- **TCP RPC for cross-cluster events** — chosen to isolate cross-cluster attack propagation latency independently of the WebSocket client path
- **Prometheus + Grafana observability stack** — enables repeatable, quantitative comparison rather than anecdotal load testing
- **Docker networking for service discovery** — required to allow the manager and cluster servers to resolve each other by service name; this was a source of bugs during construction
- **Tick rate as a key metric** — a mis-configured metric emitting at ~1 kHz instead of 20 Hz was caught and fixed, indicating tick rate stability is a first-class health signal for the benchmark

## Relationships
- [[SpatialIndex]] — the 2D grid used by the spatial baseline configuration
- [[PGP Cluster Manager]] — the social-affinity clustering implementation under test
- [[RulesEngine]] — the `arcane-rules` crate whose clustering decisions are being validated
- [[ClusterManager]] — production counterpart to the benchmark manager
- [[ClusterServer]] — production counterpart to the benchmark cluster nodes
- [[Load Generator]] — the multi-mode tool driving both configurations
- [[Prometheus Metrics]] — observability layer capturing comparison data
- [[arcane-spatial]] — crate providing `SpatialIndex` used in the spatial baseline

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]]