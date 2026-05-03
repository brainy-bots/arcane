# Architecture Decision Records (ADR)

Use this folder for **short, dated decisions** that implementers need on hand. Each ADR captures a concrete choice (integration shape, version pin, substep policy, plugin-distribution model, etc.), the alternatives considered, and the verification that the choice landed cleanly.

**Naming:** `NNN-short-slug.md` with `NNN` incrementing in filing order.

Link new ADRs from [physics-backends-and-unreal.md](../physics-backends-and-unreal.md) and any other architecture doc whose path the ADR constrains.

## Index

| # | Date | Title | Status |
|---|---|---|---|
| [001](001-rapier-cluster-integration-shape.md) | 2026-05-03 | Rapier cluster integration shape — composition over inheritance; in-process Rust; entity-keyed user API; spawn-time hooks once per entity | Accepted |
| 002 | TBD | Unreal cluster integration shape — UE-native dedicated server with Chaos; plugin distribution; UE version pin; networking implementation (C++ native vs FFI shim) | Pending — required by [`#124`](https://github.com/brainy-bots/arcane/issues/124) |
