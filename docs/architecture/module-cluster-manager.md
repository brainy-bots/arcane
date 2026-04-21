# IN-01 — ClusterManager
**Central coordinator**

---

| | |
|---|---|
| **Component ID** | IN-01 |
| **Layer** | Infrastructure |
| **Type** | Component |
| **Purpose** | Central coordinator: assign players to cluster servers, maintain spatial index and neighbor topology, invoke the clustering model, and write assignment and topology state to SpacetimeDB. Clients connect to the ClusterManager for join/leave and cluster assignment; cluster servers learn their workload and neighbors from SpacetimeDB. |
| **Document version** | 1.0 |
| **System-level companion** | [SYS-01 clustering-system-requirements.md](clustering-system-requirements.md) — ClusterManager is the orchestration component that executes on the clustering model's decisions. The SYS-01 spec describes the full decision space (capability-aware placement, market signals, etc.) this module must eventually support. |

---

## 1. Overview

ClusterManager is the single process through which all client connections (join/leave) and all clustering decisions (merge/split) flow. It does not simulate game state or run replication; it only decides *who belongs to which cluster* and *which clusters are neighbors*, and writes that state to SpacetimeDB. Cluster servers and clients then react to that state (via SpacetimeDB subscriptions or Manager messages). ClusterManager uses IClusteringModel for merge/split decisions and IServerPool to allocate or release cluster servers. It maintains a live view of world state from SpacetimeDB subscriptions and runs the clustering model on a fixed cadence. See `docs/END_TO_END_FLOWS.md` for step-by-step player join, merge, and split.

---

## 2. Responsibilities

- Accept **PLAYER_JOIN** on the Manager WebSocket (clients); validate or delegate auth; decide which cluster the player joins (existing cluster with capacity or new cluster via IServerPool.allocate()).
- **Write to SpacetimeDB** the player → cluster assignment (and server_host, server_port) so cluster servers and clients have a single source of truth.
- Send **CLUSTER_ASSIGN** to the client with cluster_id, server_host, server_port.
- Accept **PLAYER_LEAVE**; update SpacetimeDB (remove or mark player left); optionally release a cluster server if it becomes empty (IServerPool.release()).
- Maintain a **live in-memory view** of world state (cluster assignments, player positions, cluster topology, interaction signals) via SpacetimeDB subscriptions. No ad-hoc polling; the view is kept current by subscription callbacks.
- On a **fixed cadence**, call **IClusteringModel.evaluate(view)** to get merge/split decisions. Apply guardrails (confidence threshold, rate limits, resource limits). For each decision the ClusterManager agrees with: write updated assignments and topology to SpacetimeDB; send **CLUSTER_REASSIGN** to affected clients; ensure cluster servers can observe the change (they subscribe to SpacetimeDB) and, if needed, notify cluster servers to drop players/entities at end of tick (see Message Protocol).
- **Publish or write neighbor lists** so that each cluster server (or ReplicationChannelManager) knows which clusters to subscribe to for replication. Neighbor list is derived from spatial index and clustering model; stored in SpacetimeDB or pushed via a dedicated channel (see Open Questions).
- When a cluster is dissolved (merge or empty), **tear down replication subscriptions** that reference that cluster’s server, then call **IServerPool.release(server_id)**.
- Expose **Prometheus metrics** (active clusters, players, join/leave rate, merge/split rate, model eval duration, pool status).

---

## 3. What It Does NOT Do

- **Simulate game state or physics** — that is ClusterServer.
- **Run replication** (publish/subscribe between cluster servers) — that is ReplicationChannelManager and IReplicationChannel.
- **Execute game logic or RPCs** between clusters — discrete game logic (attack hit, inventory) runs in SpacetimeDB reducers; ClusterManager only handles assignment and topology. Optional RPCHandler (IN-05) is for non-game use only.
- **Store or compute game logic** — it only handles assignment and topology; authoritative game state and discrete events live in SpacetimeDB (reducers); simulation runs on ClusterServers.
- **Authenticate users** — it may delegate to an auth service or trust the client token; auth is out of scope for this component.

---

## 4. Interface / Public API

ClusterManager is a long-running process. Its “API” is the **Manager WebSocket** (for clients) and the **SpacetimeDB write surface** (for assignment and topology). It does not expose a separate RPC API for cluster servers; cluster servers learn assignments and topology from SpacetimeDB subscriptions.

### 4.1 Manager WebSocket (client-facing)

- **Listen:** WebSocket on configurable host:port (default :8081). Accept connections from clients (game instances).
- **Messages received:** PLAYER_JOIN, PLAYER_LEAVE (see §8 Message Protocol).
- **Messages sent:** CLUSTER_ASSIGN, CLUSTER_REASSIGN, SYSTEM_MESSAGE, optional METRICS_UPDATE.

### 4.2 SpacetimeDB

- **Writes:** ClusterManager is the only writer for assignment and topology tables (e.g. cluster_assignments, cluster_topology — see SpacetimeDB schema doc). It invokes reducers or direct writes to add/update/remove player → cluster assignments and cluster → neighbor lists.
- **Reads (subscriptions):** ClusterManager subscribes to the tables it needs to build the live WorldStateView (assignments, player positions, cluster metadata, interaction signals, etc.). It does not poll; subscription callbacks update the in-memory view.

### 4.3 Internal use of interfaces

- **IClusteringModel.evaluate(view) → decisions.** ClusterManager builds the view from the live state and calls evaluate on a cadence. It applies guardrails and then executes agreed decisions by writing to SpacetimeDB and sending CLUSTER_REASSIGN.
- **IServerPool.allocate() → ServerHandle.** Used when creating a new cluster (e.g. on player join when no suitable cluster exists, or when splitting). **release(server_id)** when a cluster is dissolved.

---

## 5. Internal Structure

- **Main loop or event model:** ClusterManager is event-driven. It has:
  - A **WebSocket server** for Manager connections; each client connection is tracked (player_id, connection state, current cluster_id).
  - **SpacetimeDB subscription handlers** that update the in-memory view (assignments, positions, topology, signals).
  - A **timer or tick** that fires at the evaluation cadence (e.g. every 50–200 ms). On tick: build WorldStateView from the live view, call IClusteringModel.evaluate(view), apply guardrails, for each agreed decision execute the merge or split (write SpacetimeDB, send CLUSTER_REASSIGN, tear down replication for dissolved cluster, release server if applicable).
  - Optional: a **notification path** to cluster servers (“drop these players/entities at end of tick”). If cluster servers learn purely from SpacetimeDB subscriptions, they see the assignment change and drop players/entities without an explicit message; otherwise ClusterManager sends a message (e.g. over a control channel or via a SpacetimeDB reducer that cluster servers subscribe to). See Open Questions.

- **Spatial index:** ClusterManager maintains a structure (e.g. 2D grid or spatial hash) that maps world regions or cluster centroids to cluster_ids. This is updated from the live view (player positions, cluster bounds). It is used to build the neighbor list (which clusters are “near” each other) and to feed the WorldStateView for the clustering model. The spatial index may be implemented inline in ClusterManager or delegated to a separate SpatialIndex component (IN-03); the ClusterManager is the owner of the logic that derives “who are my neighbors” from the index.

- **No blocking on cluster servers:** ClusterManager does not wait for cluster servers to acknowledge “drop” or “new assignment.” It writes to SpacetimeDB and sends to clients; cluster servers react asynchronously via subscriptions.

---

## 6. Data Ownership

- **Owns:** In-memory live view (derived from SpacetimeDB subscriptions); spatial index (or reference to IN-03); per-client connection state (player_id, cluster_id, WebSocket); evaluation cadence timer state.
- **Reads:** SpacetimeDB (subscriptions) for assignments, positions, topology, signals. IClusteringModel (evaluate). IServerPool (allocate, release, get_status).
- **Writes:** SpacetimeDB (assignment and topology tables only). Sends messages to clients over Manager WebSocket. Does not write to Redis or to cluster server processes except via SpacetimeDB or via a defined “notify cluster server” mechanism if any (see Open Questions).

---

## 7. Dependencies

| Dependency | What is used | If it changes |
|------------|--------------|----------------|
| IClusteringModel | evaluate(view) → merge/split decisions | ClusterManager must apply guardrails and execute; if the interface gains new decision types, ClusterManager must handle them. |
| IServerPool | allocate(), release(server_id), get_status() | ClusterManager must still assign players to clusters and release servers when clusters are dissolved. |
| SpacetimeDB | Subscriptions (live view), reducers or tables (writes for assignments and topology) | Schema and reducer names must match; if SpacetimeDB API changes, ClusterManager’s subscribe/write code must follow. |
| SpatialIndex (IN-03) | If separate: API for “neighbors of cluster C” or “update index with positions.” If inline: logic lives in ClusterManager. | Neighbor list and WorldStateView depend on this; merge/split triggers depend on spatial proximity. |
| ReplicationChannelManager (IN-06) | ClusterManager must ensure replication is torn down before releasing a server. Neighbor list must reach IN-06 (via SpacetimeDB or a push). | If IN-06 gets neighbor list from SpacetimeDB, ClusterManager only writes topology; if IN-06 gets a direct message, ClusterManager must send it. |

---

## 8. Message Protocol

### 8.1 Manager connection (ClusterManager ↔ client)

**Client → ClusterManager (JSON over WebSocket):**

| Message | Format | When |
|---------|--------|------|
| PLAYER_JOIN | `{ type, player_id, auth_token, position: {x,y,z}, guild_id?, party_id?, ... }` | After client opens Manager connection. |
| PLAYER_LEAVE | `{ type, player_id, reason }` | On graceful disconnect or client-initiated leave. |

**ClusterManager → Client:**

| Message | Format | When |
|---------|--------|------|
| CLUSTER_ASSIGN | `{ type, cluster_id, server_host, server_port, handoff_token? }` | After ClusterManager has written the assignment to SpacetimeDB (first join or reassignment). |
| CLUSTER_REASSIGN | `{ type, new_cluster_id, new_server_host, new_server_port, handoff_token, deadline_ms }` | When a merge or split moves this client to another cluster. |
| SYSTEM_MESSAGE | `{ type, severity, message, timestamp }` | Optional; server announcements. |
| METRICS_UPDATE | `{ type, ... }` | Optional; pushed metrics. |

### 8.2 ClusterManager → ClusterServer (notify “drop” or topology)

**Open:** Whether ClusterManager sends an explicit message to a cluster server to “drop these players/entities at end of tick” is TBD. Alternatives:

- **A. SpacetimeDB only:** Cluster servers subscribe to assignments for their cluster_id. When ClusterManager updates assignments (e.g. player P moved from B to A), server B’s subscription sees “P no longer in B” and server B drops P at end of tick. No explicit message from ClusterManager to servers.
- **B. Control channel:** ClusterManager sends a message (e.g. over TCP or a dedicated Redis topic) to each affected cluster server: “drop players [list] at end of tick.” Server acknowledges or does not; ClusterManager does not block.

Current flows doc assumes cluster servers learn from SpacetimeDB; so **A** is sufficient unless we need tighter ordering. Document in Open Questions until chosen.

### 8.3 Neighbor list delivery to ReplicationChannelManager

ClusterManager writes **cluster topology** (which clusters exist, which are neighbors) to SpacetimeDB. ReplicationChannelManager (IN-06) on each cluster server subscribes to that topology (e.g. “neighbors of my cluster_id”) and opens/closes IReplicationChannel subscriptions accordingly. So ClusterManager does not push directly to IN-06; it writes to SpacetimeDB and IN-06 reads via subscription.

---

## 9. Configuration

| Key | Default | Description |
|-----|---------|--------------|
| CLUSTER_MANAGER_HOST | 0.0.0.0 | Bind address for Manager WebSocket. |
| CLUSTER_MANAGER_PORT | 8081 | Port for Manager WebSocket. |
| CLUSTERING_EVAL_CADENCE_MS | 100 | Interval in ms between IClusteringModel.evaluate() calls. |
| HANDOFF_DEADLINE_MS | 200 | Deadline sent to clients in CLUSTER_REASSIGN; client should complete handoff within this ms. |
| SPACETIMEDB_URI | — | SpacetimeDB connection URI. |
| (See IF-01, IF-02 for model and pool config.) | | |

---

## 10. Metrics

| Metric | Type | Labels | Measures |
|--------|------|--------|----------|
| arcane_cluster_manager_active_clusters | gauge | | Number of clusters that currently have at least one assigned player/entity. |
| arcane_cluster_manager_total_players | gauge | | Total players currently assigned (across all clusters). |
| arcane_cluster_manager_joins_total | counter | | PLAYER_JOIN requests processed. |
| arcane_cluster_manager_leaves_total | counter | | PLAYER_LEAVE processed. |
| arcane_cluster_manager_assign_latency_ms | histogram | | Time from PLAYER_JOIN to CLUSTER_ASSIGN sent. |
| arcane_cluster_manager_merge_total | counter | | Merge decisions executed. |
| arcane_cluster_manager_split_total | counter | | Split decisions executed. |
| arcane_cluster_manager_eval_duration_ms | histogram | | Time spent in IClusteringModel.evaluate() per tick. |
| arcane_cluster_manager_eval_decisions_total | counter | type=merge\|split, reason= | Decisions returned by model (before guardrails). |
| arcane_cluster_manager_guardrail_rejected_total | counter | reason= | Decisions rejected by guardrails. |
| arcane_cluster_manager_pool_available | gauge | | IServerPool.get_status().available (for visibility). |

---

## 11. Failure Modes

| Failure | Detection | Response |
|---------|-----------|----------|
| SpacetimeDB unreachable | Connection or subscription error | Retry with backoff; do not accept new PLAYER_JOIN until reconnected. Existing assignments remain in SpacetimeDB; cluster servers may keep running. |
| IClusteringModel.evaluate() throws or times out | Exception or wall-clock timeout | Log; skip this evaluation cycle; do not execute any decision. Next cycle runs on cadence. Optional: fallback to static rules if ML fails repeatedly. |
| IServerPool.allocate() fails (pool exhausted) | PoolError returned | Do not create new cluster; reject or queue PLAYER_JOIN (implementation choice). Emit metric; alert. |
| Manager WebSocket client disconnect | Connection close | Treat as PLAYER_LEAVE if not already; update SpacetimeDB; optionally release cluster if empty. |
| ClusterManager process crash | — | No in-memory state; all authoritative state is in SpacetimeDB. Warm standby or new process can start, subscribe to SpacetimeDB, and resume. Clients that lost Manager connection reconnect and send PLAYER_JOIN; get CLUSTER_ASSIGN. |

---

## 12. Open Questions

- **Explicit “drop” message to cluster servers:** Use SpacetimeDB-only (servers see assignment change and drop) or add a control channel from ClusterManager to cluster servers for “drop at end of tick”? Recommendation: SpacetimeDB-only for MVP; add control channel only if ordering or latency requires it.
- **First-connect client message:** On first join, does the client send any message to the ClusterServer (e.g. “I am player P”) or does the server infer from the connection (e.g. auth token)? Affects ClusterServer spec and wire protocol.
- **SpatialIndex ownership:** Is the spatial index a separate component (IN-03) with its own API, or inline in ClusterManager? If separate, ClusterManager depends on IN-03; if inline, IN-03 doc may describe the algorithm only and ClusterManager implements it.

---

*Arcane Engine — IN-01 ClusterManager — Confidential*
