# Changelog

All notable changes to the Arcane library (Rust crates) will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Platform primitive #17:** Entity session lifecycle — first-class connect/disconnect/reconnect/leave paths with configurable persistence ladder. Games choose L0 (ephemeral), L1 (short-term reconnect, Redis TTL, default), L2 (durable, SpacetimeDB), or L3 (game-defined bucket-4 logic) via `ARCANE_PERSISTENCE` env var. See [`docs/arcane-platform-primitives.md`](docs/arcane-platform-primitives.md) #17 and epic #305.
- **Persistence environment surface:** `ARCANE_PERSISTENCE` (none | short | full, default: short), `ARCANE_RECONNECT_TTL_SECS` (default: 120 seconds), `NODE_CLIENT_IDLE_TIMEOUT_SECS`. See [`docs/architecture/progressive-api.md`](docs/architecture/progressive-api.md) §2.1.
- **Deprecated:** `SPACETIMEDB_PERSIST=1` is still honored for backwards compatibility, equivalent to `ARCANE_PERSISTENCE=full`.
- **Meta Control Layer §8.1:** Session end is now first-class lifecycle (leave path, anti-resurrection guarantee). SpacetimeDB's cold-restart role formalized as the `IPersistence` backend for L2+ games.

### Changed
- **arcane-infra**: Renamed `ClusterServer` type to `ArcaneNode`; file `cluster_server.rs` → `node.rs`. "Node" is the industry-standard term for one server in a distributed fleet; "cluster" now unambiguously means the group of entities.
- **clustering model**: The clustering decision is now a **global graph partition** (ADR-004), not a per-entity merge/split decision. `arcane_infra::manager::build_partition_decisions` derives weighted edges from the interaction graph, partitions (`GreedyGrowthPartitioner`), refines with KL/FM pair moves (`refine`), and maps partitions to cluster ids. The pluggable model seam is now `arcane_affinity::predictor::InteractionPredictor` (rule-based `HeuristicPredictor` today).
- **arcane-spatial**: `SpatialIndex` is a **3D sparse spatial hash** (was documented as a 2D grid).
- **arcane-manager env contract**: Objective weights are now tunable via `MANAGER_OBJECTIVE_ALPHA`, `MANAGER_OBJECTIVE_GAMMA`, `MANAGER_OBJECTIVE_BETA`, `MANAGER_OBJECTIVE_MU` (all optional floats; absent = library defaults per `ObjectiveWeights::default()`). Removed obsolete `MANAGER_CAPACITY_FACTOR` (capacity constraints are now encoded in the objective cost model, epic #293).

### Removed
- **workspace**: Deleted the `arcane-rules` crate (the static `IClusteringModel` `RulesEngine`). The workspace is now **6 crates** (arcane-core, arcane-affinity, arcane-spatial, arcane-pool, arcane-wire, arcane-infra).
- **arcane-core**: Removed the `IClusteringModel` trait, `ClusterDecision`, `ModelInfo`, and `evaluate()` — the manager computed and discarded the result (dead pre-partition path). The view types `WorldStateView`, `ClusterInfo`, `PlayerInfo` remain in `clustering_model`. The core contract set is now **three interfaces** (`IServerPool`, `IReplicationChannel`, `IVisibilityFilter`).
- **arcane-core**: Removed the `IWorldSimulator` trait + `world_simulator` module (no implementer or caller ever existed in-repo), `ChannelConfig` (never constructed), and the unused `EntityId` alias.
- **arcane-affinity**: Removed `AffinityEngine` + `scorer` (per-entity greedy scoring, superseded by the global partition), the `hysteresis` module (live migration cooldown is `arcane_infra::manager::MigrationState`), `capacity_factor` from `AffinityConfig` (capacity constraints are now encoded in the objective cost model), and 12 dead legacy `AffinityConfig` fields (`weight_*`, `spatial_weight`, `migration_threshold`, `cooldown_ticks`, `max_entities_per_cluster`, `capacity_soft_limit_fraction`, `merge_entity_threshold`, `physics_edge_weight`).
- **arcane-spatial**: Removed the duplicate `RadiusVisibilityFilter` (the live copy lives only in `arcane-core::visibility`).

### Performance
- **arcane-affinity**: `InteractionGraph::neighbors()` is now O(degree) via an adjacency index (was O(total pairs) per call on the router hot path). `rate_field::allocate_rates` water-level search is O(n) via suffix sums (was O(n²)).
- **arcane-spatial**: `SpatialIndex::get_neighbors` is O(N)/cycle via a cached global max weighted spread (was O(N²)).
- **arcane-infra**: `router_core::build_routing_docs` buckets `owner → entities` once, O(N) (was O(clusters × entities)); `ws_server` decodes each inbound frame once with the size guard applied before decode, and builds only the frame the active AOI mode transmits; `node_core::pump` reads the per-tick server counters once.

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
