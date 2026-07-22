# Arcane — Rust library

Multiplayer backend library: affinity clustering, node management, and replication, plus a reference server. Use this workspace for your game server or backend. Engine client plugins (Unreal Engine first) are developed in separate repositories.

**New readers:** for the positioning story — what Arcane is, who it's for, and how it compares to SpacetimeDB, Unreal/Unity dedicated servers, and traditional MMO backends — see [`WHY_ARCANE.md`](WHY_ARCANE.md).

## Crates

Six crates (`arcane-core`, `arcane-affinity`, `arcane-spatial`, `arcane-pool`, `arcane-wire`, `arcane-infra`).

| Crate | Description |
|-------|-------------|
| **arcane-core** | Traits and shared types — `IServerPool`, `IReplicationChannel`, `IVisibilityFilter`, plus the `WorldStateView` view types and `Vec2`/`Vec3`/`ClusterGeometry` (no I/O). |
| **arcane-affinity** | Interaction-weighted clustering: interaction graph, cold-pair screening, predictor, rate field, and the global graph **partition/refine** decision pipeline consumed by the manager (`build_partition_decisions`). |
| **arcane-wire** | FlatBuffers wire format — single `.fbs` schema shared by the Rust server and all engine client plugins. |
| **arcane-spatial** | SpatialIndex — in-memory 3D sparse-hash index over cluster entities for neighbor and geometry queries. |
| **arcane-pool** | LocalPool — server pool implementation. |
| **arcane-infra** | ArcaneManager, ArcaneNode, the Router, Rapier physics nodes, replication; binaries `arcane-node`, `arcane-manager`, `arcane-router`, `arcane-rapier-node`. |

## Build and test

```bash
cargo build
cargo test
```

## Architecture

See [docs/SYSTEM_ARCHITECTURE.md](docs/SYSTEM_ARCHITECTURE.md) for Mermaid diagrams of the full system: component responsibilities and how data moves between clients, ArcaneManager, Arcane Nodes, Redis, and SpacetimeDB.
See [docs/MODULE_INTERACTIONS.md](docs/MODULE_INTERACTIONS.md) for crate/module-level responsibilities and interaction boundaries inside the Rust workspace.
See [docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md](docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md) for WS/channel backpressure behavior and validation notes.

## Reference server

- **Manager** (HTTP join): `cargo run -p arcane-infra --bin arcane-manager --features manager`
- **Node** (WebSocket + Redis): `cargo run -p arcane-infra --bin arcane-node --features cluster-ws`
- **Rapier physics node**: `cargo run -p arcane-infra --bin arcane-rapier-node --features rapier-cluster`

See [arcane-demos](https://github.com/brainy-bots/arcane-demos) for a full demo (backend + client and scripts).

## Engine plugins

Engine client plugins expose Arcane through each engine's native idioms (the wire format and conceptual model are shared; the API surface is engine-native). The Unreal Engine plugin is in active development and will be published separately.

## Benchmarks

Published, reproducible scaling benchmarks live in [arcane-scaling-benchmarks](https://github.com/brainy-bots/arcane-scaling-benchmarks) — one documented command provisions, runs, and destroys the full AWS fleet.

## Star History

<a href="https://www.star-history.com/#brainy-bots/arcane&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=brainy-bots/arcane&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=brainy-bots/arcane&type=Date" />
   <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=brainy-bots/arcane&type=Date" width="600" />
 </picture>
</a>

## License

Arcane is licensed under the **GNU Affero General Public License v3.0** (AGPL-3.0). See [LICENSE](LICENSE) for the full text.

In practice:

- **You may use, modify, and distribute** the software under the AGPL-3.0, including integrating it into your own AGPL-licensed projects.
- **If you embed Arcane into a server or service that users interact with over a network**, AGPL requires you to make your modified source available to those users.
- **If you want to ship proprietary/closed-source software that links Arcane**, contact the copyright holder for a commercial license. The AGPL obligations do not apply under a commercial agreement.

For licensing inquiries: martin.mba@gmail.com

## Versioning

Releases are tagged (e.g. `v0.1.0`). See [CHANGELOG.md](CHANGELOG.md).
