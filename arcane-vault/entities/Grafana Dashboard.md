---
type: entity
tags: [grafana, observability, benchmarking, prometheus, visualization, pgp-demo, metrics]
---

# Grafana Dashboard

## What It Is
The Grafana Dashboard is the visualization layer for the Arcane PGP (Player Globe Partitioning) benchmark suite, consuming metrics from Prometheus to render real-time comparisons between clustering strategies. It serves as the empirical evidence layer — translating raw simulation telemetry (tick rates, cross-cluster attack rates, CPU utilization) into legible graphs that either confirm or refute the core Arcane hypothesis that social-affinity clustering outperforms spatial clustering.

## Origin & Evolution
The dashboard was built during the 2026-02-20 benchmark implementation session as part of a multi-component stack: Spatial Grid Server, PGP Cluster Manager, Cluster Servers, a Load Generator, and a Prometheus + Grafana monitoring pair. The goal was to produce a self-contained demonstration that could visually validate architectural claims from a specification PDF. Almost immediately the dashboard exposed fundamental data quality problems — the cluster server reported a constant 1 kHz tick rate (a metric emission bug firing at ~1 kHz instead of the intended 20 Hz), the grid server flatlined at 0 Hz, and most panels remained at zero despite the system running. The dashboard was reliable enough to surface these bugs, but the underlying simulation was too broken to produce meaningful benchmark output, eventually contributing to the session's architectural pivot away from the benchmark approach entirely.

## Technical Details
The stack runs as Docker Compose services, with Prometheus scraping metrics endpoints exposed by the Cluster Servers, Spatial Grid Server, and PGP Cluster Manager, and Grafana reading from Prometheus as its data source. Service discovery relied on Docker networking (a source of early bugs). The dashboard was designed to display side-by-side comparisons of key multiplayer simulation metrics: tick rate, cross-cluster communication volume, interaction latency, and player counts per server. A web trigger panel was added to the UI layer to manually fire player creation and interaction events, since the benchmark required explicit stimulation to produce non-zero dashboard output — a design weakness that undermined continuous benchmarking scenarios. Attempts to add real-time parameter controls (adjusting active player count and cluster configuration mid-run) failed at the UI layer, with controls resetting to zero without effect.

## Key Design Decisions
- **Prometheus as intermediary** — decouples metric collection from visualization; Grafana never scrapes services directly, allowing independent scaling and retention configuration
- **Docker Compose networking** — colocates all benchmark services for reproducibility, but introduced DNS/service-discovery bugs that required fixes before Grafana could reach Prometheus
- **Manual trigger model** — player creation and interaction events required explicit HTTP calls to produce dashboard output; this was pragmatic for demo control but proved to be an architectural liability when continuous benchmarking was needed
- **Metric emission at wrong rate** — tick rate gauge was emitting at ~1 kHz instead of 20 Hz, producing misleading constant values; the dashboard surfaced this bug before the data could be misread as valid

## Relationships
- [[Prometheus]] — primary data source; Grafana reads all time-series from Prometheus
- [[PGP Cluster Manager]] — one of the instrumented services feeding metrics
- [[Spatial Grid Server]] — baseline comparison service whose 0 Hz flatline was first visible in the dashboard
- [[Cluster Server]] — main simulation unit; tick rate bug was visible here first
- [[Load Generator]] — produces the player events that drive dashboard activity
- [[Docker Compose]] — runtime environment for the full observability stack
- [[PGP Benchmark Suite]] — the parent system the dashboard belongs to

## Conversations That Shaped This
- [[Specification implementation for concept demonstration]]
- [[Untitled Chat]]