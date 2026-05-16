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
| **arcane-infra** | ClusterManager, ArcaneNode, replication; binaries `arcane-cluster` and `arcane-manager`. |

## Build and test

```bash
cargo build
cargo test
```

## Architecture

See [docs/SYSTEM_ARCHITECTURE.md](docs/SYSTEM_ARCHITECTURE.md) for Mermaid diagrams of the full system: component responsibilities and how data moves between clients, ClusterManager, Arcane Nodes, Redis, and SpacetimeDB.
See [docs/MODULE_INTERACTIONS.md](docs/MODULE_INTERACTIONS.md) for crate/module-level responsibilities and interaction boundaries inside the Rust workspace.
See [docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md](docs/WS_CHANNEL_BACKPRESSURE_VALIDATION.md) for WS/channel backpressure behavior and validation notes.

## Reference server

- **Manager** (HTTP join): `cargo run -p arcane-infra --bin arcane-manager --features manager`
- **Cluster** (WebSocket + Redis): `cargo run -p arcane-infra --bin arcane-cluster --features cluster-ws`

See [arcane-demos](https://github.com/brainy-bots/arcane-demos) for a full demo (backend + Unreal client and scripts).

## Unreal client

The Unreal Engine client plugin lives in a separate repo: **arcane-client-unreal**. Add it to your project's `Plugins/` folder.

## Development vault

The [`arcane-vault/`](arcane-vault/) directory is an [Obsidian](https://obsidian.md/) knowledge vault that documents how Arcane was built. It was generated from the full history of AI coding sessions (Cursor IDE + Claude Code) using an LLM-powered pipeline inspired by [Karpathy's LLM-wiki approach](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f).

**Contents:**

| Folder | What's in it |
|--------|-------------|
| `conversations/` | One distilled note per coding session — summary, key decisions, problems solved, entities mentioned |
| `entities/` | Concept pages for every system, interface, and component — cross-linked with `[[wikilinks]]` |
| `timeline.md` | Chronological narrative of the project from first spec to current state |
| `00-INDEX.md` | Master navigation table |
| `SCHEMA.md` | Vault conventions and how to update |

**Opening in Obsidian:**

1. Install [Obsidian](https://obsidian.md/) (free for local use)
2. **Open folder as vault** → select `arcane-vault/`
3. Open **Graph view** (left sidebar) to explore the concept map — 20 conversation nodes + 99 entity nodes, all interconnected

**Regenerating the vault:**

The vault is built by `arcane-vault-builder.py` in the repo root. It reads chat exports from `arcane-scaling-benchmarks/` and READMEs from all Arcane repos, then calls the Anthropic API to summarize and cross-link everything.

```bash
pip install anthropic
export ANTHROPIC_API_KEY=<your-key>
python arcane-vault-builder.py            # estimate cost
python arcane-vault-builder.py --sample   # process one file (sanity check)
python arcane-vault-builder.py --confirm  # full build (~$10, ~20 min)
```

Intermediate results are cached in `.vault-build/` (gitignored). Re-runs skip already-processed files.

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
