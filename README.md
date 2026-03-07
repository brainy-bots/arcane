# Arcane — Rust library

Multiplayer backend library: cluster management, replication, and reference server. Use this crate for your game server or backend; use **arcane-client-unreal** for the Unreal Engine client plugin.

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

## Reference server

- **Manager** (HTTP join): `cargo run -p arcane-infra --bin arcane-manager --features manager`
- **Cluster** (WebSocket + Redis): `cargo run -p arcane-infra --bin arcane-cluster --features cluster-ws`

See [arcane-demos](https://github.com/brainy-bots/arcane-demos) for a full demo (backend + Unreal client and scripts).

## Unreal client

The Unreal Engine client plugin lives in a separate repo: **arcane-client-unreal**. Add it to your project's `Plugins/` folder.

## Versioning

Releases are tagged (e.g. `v0.1.0`). See [CHANGELOG.md](CHANGELOG.md).
