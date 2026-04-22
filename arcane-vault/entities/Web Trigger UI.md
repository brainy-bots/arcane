---
type: entity
tags: [ui, web-trigger, benchmarking, pgp, load-testing, monitoring, docker, visualization]
---

# Web Trigger UI

## What It Is
The Web Trigger UI is a browser-based control panel built during the PGP cluster benchmarking work, providing a visual interface for launching and managing benchmark scenarios against the Arcane backend. It allows operators to trigger load generation modes, initiate clustering scenarios, and observe system state without dropping to the command line. It serves as the human-facing surface of the benchmark and demonstration stack.

## Origin & Evolution
The Web Trigger UI emerged from the PGP (Player Globe Partitioning) benchmark session on 2026-02-20, where the goal was to empirically validate social-affinity clustering versus spatial clustering. As the benchmark suite grew more complex — spanning a Spatial Grid Server, PGP Cluster Manager, individual Cluster Servers, and a multi-mode Load Generator — the need for a coordinated control surface became apparent. The UI layer evolved substantially over the course of that session, starting as a simple trigger panel and expanding as more benchmark modes and monitoring integrations were added alongside the Prometheus + Grafana stack.

## Technical Details
The Web Trigger UI is part of the broader benchmark and demonstration infrastructure rather than the core Arcane library crates. It sits alongside the Docker-composed monitoring stack and communicates with backend services (the Load Generator and Cluster Manager) via HTTP. The UI is designed to surface the multi-mode nature of the Load Generator, allowing selection between benchmark scenarios (e.g., spatial baseline vs. PGP clustering). It lives in the `arcane-scaling-benchmarks` workspace, separate from the main `arcane` Rust library workspace.

## Key Design Decisions
- **Browser-based, not CLI** — rationale: operators running the benchmark stack via Docker Compose needed a low-friction way to trigger scenarios without maintaining shell sessions into containers
- **Wired to the Load Generator's HTTP interface** — rationale: the Load Generator already exposed HTTP coordination endpoints used by the Cluster Manager; the UI reuses the same surface rather than introducing a separate control protocol
- **Bundled with the monitoring stack** — rationale: Prometheus and Grafana were already part of the Docker Compose topology, so the trigger panel was co-located to give a unified operator experience in one browser session

## Relationships
- [[PGP Cluster Manager]]
- [[Load Generator]]
- [[Spatial Grid Server]]
- [[Cluster Server]]
- [[Prometheus + Grafana Monitoring Stack]]
- [[Docker Compose Benchmark Stack]]
- [[Player Globe Partitioning (PGP)]]

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]]