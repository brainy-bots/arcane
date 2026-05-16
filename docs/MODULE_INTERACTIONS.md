# Arcane module interactions

This document complements `SYSTEM_ARCHITECTURE.md` by focusing on crate/module boundaries inside the Rust workspace.

**Entity state and physics:** Where fields live on the wire vs in SpacetimeDB is specified in [architecture/four-bucket-state-model.md](architecture/four-bucket-state-model.md). How authoritative physics integrates with the cluster tick (including Unreal Chaos) is in [architecture/physics-backends-and-unreal.md](architecture/physics-backends-and-unreal.md).

## Workspace-level module graph

```mermaid
flowchart LR
  subgraph Core["arcane-core (contracts)"]
    Types["types"]
    Model["clustering_model"]
    PoolIf["server_pool"]
    ReplIf["replication_channel"]
    SimIf["world_simulator"]
  end

  subgraph Spatial["arcane-spatial"]
    SIndex["SpatialIndex"]
  end

  subgraph Rules["arcane-rules"]
    RulesEngine["RulesEngine"]
  end

  subgraph Pool["arcane-pool"]
    LocalPool["LocalPool"]
  end

  subgraph Infra["arcane-infra"]
    Manager["cluster_manager"]
    Server["node"]
    Runner["cluster_runner"]
    Ws["ws_server"]
    ReplMgr["replication_channel_manager"]
    Neighbor["neighbor_subscriber"]
    Persist["spacetimedb_persist"]
    Redis["redis_channel"]
    Rpc["rpc_handler"]
  end

  Types --> SIndex
  Model --> RulesEngine
  PoolIf --> LocalPool
  ReplIf --> Redis
  ReplIf --> ReplMgr
  ReplIf --> Neighbor
  SimIf --> Server

  SIndex --> Manager
  RulesEngine --> Manager
  LocalPool --> Manager

  Manager --> Runner
  Server --> Runner
  ReplMgr --> Runner
  Neighbor --> Runner
  Ws --> Runner
  Persist --> Runner
  Rpc --> Runner
```

## Runtime interaction highlights

- `cluster_runner` is the integration point: it wires simulation (`node`), inbound neighbor replication (`neighbor_subscriber`), outbound client transport (`ws_server`), and optional persistence (`spacetimedb_persist`).
- `cluster_manager` is control-plane focused and depends on abstractions (`IClusteringModel`, `IServerPool`) implemented by `arcane-rules` and `arcane-pool`.
- `arcane-core` remains dependency-root only (no transport, I/O, or process orchestration code).
