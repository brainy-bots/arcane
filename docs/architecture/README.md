# Architecture Specs Index

This folder contains interface and module responsibility specs for the `arcane` repository.

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

- `module-cluster-manager.md`
- `module-cluster-server.md`
- `module-spatial-index.md`
- `module-rules-engine.md`
- `module-rpc-handler.md`
- `module-replication-channel-manager.md`

## Mapping

- `component-index.md` provides cross-component mapping.

Note: these docs originated as initial system specs and should be kept aligned with implementation updates.
