# Architecture Specs Index

This folder contains interface and module responsibility specs for the `arcane` repository.

## Design pillars

- **[progressive-api.md](progressive-api.md)** — Every platform capability is a ladder: level-0 free default, level-1 data-level, level-2 simple knob, level-3 typed opt-in, level-4 escape hatch. Complexity paid ≈ optimization gained. Contributors adding new APIs follow this shape; reviewers push back when a PR forces advanced APIs on users who don't need them yet.

## System requirements

- **[clustering-system-requirements.md](clustering-system-requirements.md)** — System-level spec for what the clustering system must eventually do end-to-end: joint optimization over player grouping, capability-aware placement (instance type, AZ, cost class), temporal prediction, and cost optimization. Companion to IF-01, which defines the interface; this one defines the responsibility envelope. Cites the benchmark evidence that drives the roadmap.

## System data flow

- **[connection-types.md](connection-types.md)** — The five connection types in an Arcane deployment: Client→Cluster (WebSocket), Client→SpacetimeDB (actions), SpacetimeDB→Cluster (subscriptions), Cluster→Cluster (Redis), Cluster→SpacetimeDB (persistence). What flows through each and why.

## Entity state model and physics

- **[four-bucket-state-model.md](four-bucket-state-model.md)** — Where entity data lives (spine, replicated JSON, cluster-local, SpacetimeDB) and how it maps to `EntityStateEntry` and SpacetimeDB.
- **[physics-backends-and-unreal.md](physics-backends-and-unreal.md)** — Integrating authoritative physics (Unreal Chaos first, optional Rust backends): `ClusterSimulation`, tick order, recommended server layout, checklists.
- **`adr/`** — Architecture decision records (e.g. chosen Unreal integration shape, UE version). See [adr/README.md](adr/README.md).

## Interfaces

- `interface-iclusteringmodel.md`
- `interface-iserverpool.md`
- `interface-ireplicationchannel.md`
- `interface-iworldsimulator.md`

## Modules

- `module-arcane-manager.md`
- `module-arcane-node.md`
- `module-spatial-index.md`
- `module-rules-engine.md`
- `module-rpc-handler.md`
- `module-replication-channel-manager.md`

## Mapping

- `component-index.md` provides cross-component mapping.

Note: these docs originated as initial system specs and should be kept aligned with implementation updates.
