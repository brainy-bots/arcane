# Arcane — Rust library

Multiplayer backend library: cluster management, replication, and reference server. Use this crate for your game server or backend; use **arcane-client-unreal** for the Unreal Engine client plugin.

**New readers:** for the positioning story — what Arcane is, who it's for, and how it compares to SpacetimeDB, Unreal/Unity dedicated servers, and traditional MMO backends — see [`WHY_ARCANE.md`](WHY_ARCANE.md).

## Crates

| Crate | Description |
|-------|-------------|
| **arcane-core** | Traits and shared types (no I/O). |
| **arcane-spatial** | SpatialIndex — 2D grid for neighbor discovery. |
| **arcane-rules** | RulesEngine — clustering decisions. |
| **arcane-pool** | LocalPool — server pool implementation. |
| **arcane-infra** | ClusterManager, ClusterServer, replication; binaries `arcane-cluster` and `arcane-manager`. |

## Build and test

```bash
cargo build
cargo test
```

## Architecture

See [docs/SYSTEM_ARCHITECTURE.md](docs/SYSTEM_ARCHITECTURE.md) for Mermaid diagrams of the full system: component responsibilities and how data moves between clients, ClusterManager, ClusterServers, Redis, and SpacetimeDB.
See [docs/MODULE_INTERACTIONS.md](docs/MODULE_INTERACTIONS.md) for crate/module-level responsibilities and interaction boundaries inside the Rust workspace.
See [docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md](docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md) for WS/channel backpressure behavior and validation notes.

## Reference server

- **Manager** (HTTP join): `cargo run -p arcane-infra --bin arcane-manager --features manager`
- **Cluster** (WebSocket + Redis): `cargo run -p arcane-infra --bin arcane-cluster --features cluster-ws`

See [arcane-demos](https://github.com/brainy-bots/arcane-demos) for a full demo (backend + Unreal client and scripts).

## Unreal client

The Unreal Engine client plugin lives in a separate repo: **arcane-client-unreal**. Add it to your project's `Plugins/` folder.

## License

Arcane is licensed under the **GNU Affero General Public License v3.0** (AGPL-3.0). See [LICENSE](LICENSE) for the full text.

In practice:

- **You may use, modify, and distribute** the software under the AGPL-3.0, including integrating it into your own AGPL-licensed projects.
- **If you embed Arcane into a server or service that users interact with over a network**, AGPL requires you to make your modified source available to those users.
- **If you want to ship proprietary/closed-source software that links Arcane**, contact the copyright holder for a commercial license. The AGPL obligations do not apply under a commercial agreement.

For licensing inquiries: martin.mba@gmail.com

## Versioning

Releases are tagged (e.g. `v0.1.0`). See [CHANGELOG.md](CHANGELOG.md).
