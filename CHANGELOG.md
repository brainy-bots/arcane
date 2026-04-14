# Changelog

All notable changes to the Arcane library (Rust crates) will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- **arcane-core**: `EntityStateEntry` now carries **`user_data`** and **`local_data`** (JSON) aligned with the [four-bucket state model](docs/architecture/four-bucket-state-model.md): replicated simulation payload vs cluster-local fields; `local_data` is never serialized on `EntityStateDelta`. Added `EntityStateEntry::new`.
- **arcane-infra**: WebSocket `PLAYER_STATE` accepts optional **`user_data`**; **`local_data`** is never read from clients.

## [0.1.0] - (initial split)

- **arcane-core**: Traits and shared types (IClusteringModel, IServerPool, IReplicationChannel, IWorldSimulator; Vec2, Vec3, ClusterGeometry, WorldStateView, etc.).
- **arcane-spatial**: SpatialIndex (IN-03) — 2D coarse grid for neighbor discovery.
- **arcane-rules**: RulesEngine (IN-04) — static IClusteringModel implementation.
- **arcane-pool**: LocalPool / ClusterServerPool (IN-07) — IServerPool implementation.
- **arcane-infra**: ClusterManager, ClusterServer, ReplicationChannelManager, RPCHandler; binaries `arcane-cluster` and `arcane-manager` (reference server).

[Unreleased]: https://github.com/brainy-bots/arcane/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/brainy-bots/arcane/releases/tag/v0.1.0
