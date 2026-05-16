# Changelog

All notable changes to the Arcane library (Rust crates) will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- **arcane-infra**: Renamed `ClusterServer` type to `ArcaneNode`; file `cluster_server.rs` → `node.rs`. "Node" is the industry-standard term for one server in a distributed fleet; "cluster" now unambiguously means the group of entities.

## [0.2.0] - 2026-04-18

### Added
- **arcane-core**: `GameAction` struct for client-to-cluster game action messages (entity_id, action_type, JSON payload).
- **arcane-core**: `ClusterTickContext::game_actions` field — simulation receives client actions each tick.
- **arcane-infra**: WebSocket server parses `GAME_ACTION` messages alongside `PLAYER_STATE`, routes to separate channel.
- **arcane-infra**: `cluster_runner` drains game actions per tick and passes to `simulate_before_tick`.
- **docs**: Connection types architecture doc — the four connection types in an Arcane deployment and developer decision guide.

## [0.1.0] - 2026-04-17

### Added
- **arcane-core**: `EntityStateEntry` now carries **`user_data`** and **`local_data`** (JSON) aligned with the [four-bucket state model](docs/architecture/four-bucket-state-model.md): replicated simulation payload vs cluster-local fields. Added `EntityStateEntry::new`.
- **arcane-core**: `ClusterSimulation` trait and `ClusterTickContext` — pluggable per-tick simulation hook for cluster-owned entities. Games implement this trait to inject physics/game logic into the cluster tick loop.
- **arcane-infra**: `ClusterServer::simulate_before_tick()` — runs custom simulation with exclusive entity access before building the replication delta.
- **arcane-infra**: `run_cluster_loop` accepts `Option<Arc<dyn ClusterSimulation>>`.
- **arcane-infra**: `ClusterServer::with_max_entities()` — configurable entity map cap (default 100K) to prevent unbounded memory growth.
- **arcane-infra**: WebSocket `PLAYER_STATE` accepts optional **`user_data`** (bucket 2); **`local_data`** is never read from clients.
- **arcane-infra**: Input validation — reject NaN/Infinity positions and velocities; cap message size to 64 KiB.

### Fixed
- **arcane-core**: `local_data` uses **`skip_deserializing`** so replication/Redis JSON cannot inject bucket-3 state.

### Removed
- **arcane-infra**: Removed unused `ClusterServer::run()` and `ClusterManager::run()` stub methods.

## [0.0.1] - (initial split)

- **arcane-core**: Traits and shared types (IClusteringModel, IServerPool, IReplicationChannel, IWorldSimulator; Vec2, Vec3, ClusterGeometry, WorldStateView, etc.).
- **arcane-spatial**: SpatialIndex (IN-03) — 2D coarse grid for neighbor discovery.
- **arcane-rules**: RulesEngine (IN-04) — static IClusteringModel implementation.
- **arcane-pool**: LocalPool / ClusterServerPool (IN-07) — IServerPool implementation.
- **arcane-infra**: ClusterManager, ClusterServer, ReplicationChannelManager, RPCHandler; binaries `arcane-cluster` and `arcane-manager` (reference server).

[Unreleased]: https://github.com/brainy-bots/arcane/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/brainy-bots/arcane/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/brainy-bots/arcane/releases/tag/v0.1.0
[0.0.1]: https://github.com/brainy-bots/arcane/releases/tag/v0.0.1
