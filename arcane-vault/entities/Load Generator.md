---
type: entity
tags: [load-generator, benchmarking, load-testing, websocket, rust, pgp, clustering, simulation]
---

# Load Generator

## What It Is
The Load Generator is a multi-mode traffic simulation tool built for the Arcane PGP (Player Globe Partitioning) benchmark suite. It simulates player client connections and behavior at scale, enabling empirical validation of clustering strategies by driving WebSocket load against the backend infrastructure. It plays a central role in measuring cross-cluster communication characteristics under realistic gameplay patterns.

## Origin & Evolution
The Load Generator was built from scratch during the 2026-02-20 PGP benchmark session, driven by a PDF specification intended to empirically test whether social-affinity clustering (guilds, parties) outperforms spatial clustering for cross-cluster traffic. The benchmark suite required a realistic client simulator capable of operating in multiple modes to isolate different variables — social groupings, spatial distributions, attack patterns — so the Load Generator was designed with multiple modes from the outset rather than being extended incrementally.

## Technical Details
The Load Generator connects to cluster servers via WebSocket, simulating player clients at scale. It operates in multiple modes to support different benchmark scenarios — including spatial-baseline mode (players distributed by position) and PGP/social-affinity mode (players grouped by guild or party membership). It is part of a broader benchmark stack alongside a Spatial Grid Server, a PGP Cluster Manager (HTTP coordination), individual Cluster Servers (WebSocket + TCP RPC for cross-cluster attacks), and a Prometheus + Grafana monitoring stack. Docker networking is used for service discovery across the stack. Known issues encountered during development included incorrect RPC failure reporting and a tick rate metric emitting at ~1 kHz instead of the intended 20 Hz, both of which were resolved during the session.

## Key Design Decisions
- **Multi-mode design from the start** — rather than a single traffic pattern, multiple modes were built in to isolate social-affinity vs. spatial clustering variables without requiring separate tools
- **WebSocket as the simulation transport** — matches the actual Arcane client protocol, making load behavior representative of real player connections
- **Part of a containerized benchmark stack** — deployed alongside monitoring (Prometheus + Grafana) and cluster infrastructure via Docker, enabling reproducible runs and proper service discovery

## Relationships
- [[PGP Cluster Manager]]
- [[Cluster Server]]
- [[Spatial Grid Server]]
- [[SpatialIndex]]
- [[ClusterManager]]
- [[arcane-infra]]
- [[Prometheus]]
- [[Grafana]]

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]]