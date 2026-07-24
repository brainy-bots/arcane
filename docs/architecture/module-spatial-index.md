# IN-03 — SpatialIndex
**3D sparse spatial hash for neighbor discovery and proximity**

---

> **Reconciled to code (2026-07).** The live index is a **3D sparse spatial hash** over cluster centroids, not a 2D grid (`crates/arcane-spatial/src/index.rs:1-17`). A configurable vertical weight `y_weight` tunes the metric; `y_weight = 0.0` reproduces the legacy 2D behavior. `get_neighbors` is **O(N)** (cached max-spread over occupied cells), not O(N²). The index is consumed by the **ArcaneManager**, which snapshots it into the `WorldStateView` for the global partition (`build_partition_decisions`); the former `IClusteringModel.evaluate()` seam was removed (see [ADR-004](adr/004-global-partitioning-and-ml-seams.md)). The 2D framing below is retained as original design intent; read "2D grid" as the `y_weight = 0` special case of the 3D hash.

---

| | |
|---|---|
| **Component ID** | IN-03 |
| **Layer** | Infrastructure |
| **Type** | Component (library or module) |
| **Purpose** | Maintain a 3D sparse spatial hash over cluster entities (positions + cluster ownership), with a configurable vertical weight (`y_weight`) whose `0.0` setting reproduces legacy 2D behavior. Expose queries for cluster centroid, spread radius, and which clusters are neighbors (effective area overlap). Feeds ArcaneManager’s neighbor list and the WorldStateView snapshot used by the global partition. Data that populates the index originates from SpacetimeDB (entity_state, entity_assignments) written by Arcane Nodes; the index is updated by the component that holds the live view (ArcaneManager). |
| **Document version** | 1.0 |

---

## 1. Overview

SpatialIndex is a data structure and query API used to answer: (1) Where is each cluster in the world (centroid, spread)? (2) Which clusters are “neighbors” (close enough that they might need to replicate state or be merge candidates)? Neighbor definition uses the same formula as replication filtering: **centroid + spread_radius + observation_radius** (see IF-03). The index is coarse (e.g. grid cells or spatial hash buckets) to keep updates and queries cheap; it does not store full entity state, only what is needed for proximity and neighbor discovery.

The index is **updated** by the process that has the live world view — in this architecture, ArcaneManager, which subscribes to SpacetimeDB (entity_state, entity_assignments) and receives position updates. The **underlying position data** is written by Arcane Nodes (they call upsert_entity_state). So “updated by cluster servers” means the data source is cluster servers via SpacetimeDB; the SpatialIndex component itself is updated by ArcaneManager when subscription callbacks fire.

SpatialIndex has no external dependencies (no SpacetimeDB, no Redis). It is a pure in-memory structure. ArcaneManager (or another single consumer) owns one instance and feeds it; no other process updates it.

---

## 2. Responsibilities

- **Store per-cluster aggregate geometry:** For each cluster_id, maintain centroid (e.g. mean position of its entities) and spread_radius (max distance of any entity from centroid). Updated whenever entity positions for that cluster change (from live view).
- **Support spatial lookup:** Map world positions (or regions) to cluster_ids. Structure: a 3D sparse spatial hash (cell size on the order of the observation radius) with a weighted vertical axis (`y_weight`; `0.0` degenerates to 2D). Used for “which clusters have entities in this region?” and “which clusters overlap this cluster’s effective area?”
- **Compute neighbor set:** Given cluster_id and observation_radius, return the set of cluster_ids that are “neighbors”: clusters whose effective area (centroid + spread_radius + observation_radius) overlaps or is within range of this cluster’s effective area. Used by ArcaneManager to write cluster_topology.neighbor_ids and by ReplicationChannelManager (via topology) to decide which replication subscriptions to open.
- **Support WorldStateView:** Provide cluster list with centroid, spread, entity count (and optionally player positions) so ArcaneManager can build the WorldStateView passed to the global partition (`build_partition_decisions`). May be a separate view builder that uses the index plus assignment data; the index is the spatial part.
- **Efficient updates:** Support incremental update: add/update/remove entity (entity_id, cluster_id, position). Recompute affected cluster’s centroid and spread_radius; update grid/hash cells. No full rebuild on every tick.

---

## 3. What It Does NOT Do

- **Fetch data from SpacetimeDB or any network** — It does not subscribe or connect. The caller (ArcaneManager) feeds it.
- **Decide merge/split** — It only answers spatial queries. The ArcaneManager's global partition (`build_partition_decisions`) makes the clustering decisions.
- **Manage replication subscriptions** — ReplicationChannelManager uses neighbor lists (from topology); the index only produces those lists.
- **Store full entity state** — Only position (and cluster_id) per entity for index purposes; centroid and spread are derived. Full state lives in SpacetimeDB and in ArcaneNode memory.

---

## 4. Interface / Public API

Language-agnostic. Implementations may be in-process (ArcaneManager calls directly) or a library crate used by ArcaneManager.

### 4.1 Update (caller feeds data)

```
update_entity(entity_id: ID, cluster_id: ID, position: (x, y, z)) -> void
```

Register or update an entity’s position and cluster. If the entity moved clusters (cluster_id changed), the previous cluster’s centroid/spread must be updated (remove entity from old cluster), and the new cluster’s (add entity). Caller is responsible for consistency with entity_assignments.

```
remove_entity(entity_id: ID, cluster_id: ID) -> void
```

Remove an entity from the index (e.g. despawn or reassignment). Updates that cluster’s centroid and spread.

```
set_observation_radius(radius: float) -> void
```

Set the observation radius used for neighbor queries. Typically called once at init or from config.

### 4.2 Query

```
get_cluster_geometry(cluster_id: ID) -> { centroid: (x,y,z), spread_radius: float, entity_count: int } | None
```

Return current centroid, spread radius, and entity count for a cluster. Returns None if cluster has no entities in the index.

```
get_neighbors(cluster_id: ID) -> ID[]
```

Return cluster_ids whose effective area (centroid + spread_radius + observation_radius) overlaps or is within range of the given cluster’s effective area. Used to derive cluster_topology.neighbor_ids. Symmetric: if A is neighbor of B, B is neighbor of A (when both have up-to-date geometry).

```
get_clusters_in_region(center: (x,y), radius: float) -> ID[]
```

Optional. Return cluster_ids that have any entity (or centroid) in the given 2D region. Used for region-based queries or debugging.

```
snapshot_for_view() -> ClusterGeometry[]
```

Return a snapshot of all clusters with centroid, spread_radius, entity_count (and optionally list of entity positions) for building WorldStateView. Called by ArcaneManager before it runs the global partition (`build_partition_decisions`).

```
ClusterGeometry { cluster_id, centroid, spread_radius, entity_count, player_ids?, positions? }
```

Exact shape is defined by what the `WorldStateView` view types expect (`arcane-core::clustering_model`).

---

## 5. Internal Structure

- **Per-cluster state:** Map cluster_id → { centroid (running sum/count or recomputed), spread_radius (max distance from centroid), set of entity_ids with positions }. Centroid and spread can be updated incrementally: on add/remove entity, update sum and count, then centroid = sum/count; spread_radius = max over entity distances from new centroid (or approximate with running max).
- **Spatial structure:** 3D sparse hash. Key = weighted cell `(cell_x, cell_y, cell_z)`; value = set of cluster_ids that have at least one entity in that cell. The vertical axis is scaled by `y_weight` before cell mapping, so `y_weight = 0.0` collapses the z-term and reproduces the legacy 2D grid. On entity update: remove from old cell(s), add to new cell(s). Cell size is on the order of observation_radius so that neighbor queries do not require scanning many cells. For “get neighbors of C,” get C’s effective radius (spread_radius + observation_radius), find all cells overlapping a sphere (weighted) centered at C’s centroid with that radius; collect unique cluster_ids in those cells; optionally filter by actual distance for precision. Queries touch only occupied cells within the radius using cached geometry — O(N), not O(N²) (`index.rs:1-17`).
- **Thread safety:** If ArcaneManager is single-threaded, no locking. If subscription callbacks run on another thread, the index must be updated in a thread-safe way (e.g. queue updates and apply on ArcaneManager’s tick, or use a concurrent structure). Document the chosen model.

---

## 6. Data Ownership

- **Owns:** In-memory grid/hash, per-cluster centroid/spread/entity sets, observation_radius.
- **Reads:** Nothing (no I/O). Caller supplies all data via update_entity / remove_entity.
- **Writes:** Nothing external. Only internal state.

---

## 7. Dependencies

| Dependency | What is used | If it changes |
|------------|--------------|--------------|
| None | — | SpatialIndex is a standalone library. ArcaneManager depends on it and feeds it from SpacetimeDB subscription data. |

---

## 8. Message Protocol

Not applicable. SpatialIndex is not a network service; it has no message protocol.

---

## 9. Configuration

| Key | Default | Description |
|-----|---------|--------------|
| SPATIAL_INDEX_CELL_SIZE | observation_radius or 2× | Grid cell size in world units. Larger = fewer cells, coarser; smaller = more precision, more memory. |
| SPATIAL_INDEX_OBSERVATION_RADIUS | Same as replication (e.g. 200.0) | Used in get_neighbors() effective area. Should match IF-03 observation_radius. |

Typically ArcaneManager passes observation_radius from config into set_observation_radius().

---

## 10. Metrics

SpatialIndex may expose metrics if used by ArcaneManager (ArcaneManager could report them under its own metrics). Optional:

| Metric | Type | Labels | Measures |
|--------|------|--------|----------|
| arcane_spatial_index_cluster_count | gauge | | Number of clusters in the index. |
| arcane_spatial_index_entity_count | gauge | | Total entities in the index. |
| arcane_spatial_index_update_duration_us | histogram | | Time to apply update_entity / remove_entity. |
| arcane_spatial_index_neighbor_query_duration_us | histogram | | Time for get_neighbors() call. |

If implemented inline in ArcaneManager, these can be merged into ArcaneManager metrics.

---

## 11. Failure Modes

| Failure | Detection | Response |
|---------|-----------|----------|
| Stale data | Caller stops feeding updates | Index becomes stale; neighbor lists and WorldStateView wrong until caller resumes. No internal recovery; caller (ArcaneManager) must keep feeding from SpacetimeDB. |
| Invalid input (e.g. NaN position) | update_entity with bad position | Document: behavior is undefined or clamp/skip. Caller should validate. |
| Memory growth | Many entities/clusters | Index size is O(entities + clusters). Bounded by world population. No explicit cap; monitor entity_count. |

---

## 12. Open Questions

- **3D vs 2D (RESOLVED in code):** The live index is a 3D sparse hash with a configurable vertical weight `y_weight` (`index.rs:12-17`). `y_weight = 1.0` is a spherical metric, larger values shrink the effective vertical range (the AOI-cylinder shape most games want), and `y_weight = 0.0` reproduces the legacy 2D (x, z) behavior. `ClusterGeometry.spread_radius` stays in unweighted world units as a public-API contract.
- **Where it runs:** Always in-process with ArcaneManager in this design. If ArcaneManager were scaled out, only one instance would own the index (or each instance would have its own from its view); no distributed index in scope.

---

*Arcane Engine — IN-03 SpatialIndex — Confidential*
