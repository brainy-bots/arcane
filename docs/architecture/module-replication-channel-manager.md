# IN-06 — ReplicationChannelManager
**Manage replication subscriptions per cluster**

---

| | |
|---|---|
| **Component ID** | IN-06 |
| **Layer** | Infrastructure |
| **Type** | Component |
| **Purpose** | Runs on each ClusterServer. Subscribes to SpacetimeDB cluster_topology to get this cluster’s neighbor list; opens and closes IReplicationChannel instances (one per neighbor). Delivers outbound deltas from ClusterServer to each channel and inbound deltas from channels to ClusterServer. Does not decide who is a neighbor — topology comes from ClusterManager via SpacetimeDB. |
| **Document version** | 1.0 |

---

## 1. Overview

ReplicationChannelManager is the component that keeps replication subscriptions in sync with cluster topology. It does not run as a separate process; it runs inside each ClusterServer process. It reads the neighbor list (and optionally per-neighbor centroid/spread for observation-radius filtering) from SpacetimeDB `cluster_topology`; when the topology changes (merge, split, or ClusterManager refresh), it opens new IReplicationChannel instances for new neighbors and closes channels for neighbors that are no longer in the list. ClusterServer calls ReplicationChannelManager to send outbound deltas and receives inbound deltas via a callback or queue. See IF-03 for the IReplicationChannel contract and Redis pub/sub behavior; see `in_02_cluster_server.md` for how ClusterServer uses replication.

---

## 2. Responsibilities

- **Subscribe to SpacetimeDB cluster_topology:** Subscribe to the row(s) that define this cluster’s identity and neighbors — e.g. the row where `cluster_id = my_cluster_id` (and optionally the rows for each neighbor to get endpoint info and centroid/spread). When ClusterManager updates `cluster_topology.neighbor_ids` (or neighbor rows), the subscription delivers the new state.
- **Maintain the set of IReplicationChannel instances:** One channel per current neighbor. When the topology says “neighbors = [A, B, C]” and the previous set was [A, B], open a channel for C and close none. When the topology says “neighbors = [A]” (e.g. after a merge), close channels for B and C. When a cluster is dissolved (e.g. B merged into A), ClusterManager removes B from topology and all servers that had B as neighbor see the update and close the channel to B.
- **Open channels:** For each new neighbor, call `IReplicationChannel.open(source_cluster_id = my_cluster_id, destination = neighbor’s ServerHandle, config)`. The implementation (e.g. RedisPubSubReplication) subscribes to that neighbor’s topic; this cluster’s state is published to its own topic (neighbors subscribe to us). Store the channel and the neighbor’s cluster_id and endpoint (host/port for TCP, or topic name for Redis).
- **Close channels:** When a neighbor is removed from the list or the cluster is shutting down, call `IReplicationChannel.close(reason)`. Reason can be NEIGHBOR_DEPARTED, CLUSTERS_MERGED, or SHUTDOWN. Flush and unsubscribe; do not leave orphan subscriptions.
- **Provide destination geometry to channels:** For observation-radius filtering (IF-03), each channel needs the destination cluster’s centroid and spread_radius. ReplicationChannelManager obtains these from cluster_topology if stored (optional columns), or from a separate subscription/view, or from a default. Push updates to each channel when topology or geometry changes so the send path can filter entities correctly.
- **Send path:** Expose a method for ClusterServer to submit an outbound EntityStateDelta (e.g. `send_to_all_neighbors(delta)` or `get_channels() -> [IReplicationChannel]` and ClusterServer calls `send(delta)` on each). ReplicationChannelManager does not build the delta; ClusterServer builds it and passes it. Each channel may apply per-destination filtering (observation radius) using the destination geometry provided above.
- **Receive path:** When a channel receives a message from a subscription (Redis callback or TCP read), decode the delta and deliver it to ClusterServer (e.g. `on_receive(source_cluster_id, delta)` callback or a queue that ClusterServer drains each tick). ClusterServer merges into its neighbor-entity cache and handles gap detection (IF-03).
- **Reconnect handling:** IReplicationChannel implementations reconnect or resubscribe on broker failure. ReplicationChannelManager does not need to recreate channels unless the topology changed; it may expose channel status (connected/disconnected) for metrics or logging.
- **Expose metrics:** Subscription count, per-channel send/receive rates, drop counts (from channel get_status() or equivalent). See IF-03 for per-channel metrics; ReplicationChannelManager may aggregate or re-export them with cluster_id label.

---

## 3. What It Does NOT Do

- **Decide which clusters are neighbors** — ClusterManager (and IClusteringModel / SpatialIndex) decide; ClusterManager writes cluster_topology. ReplicationChannelManager only reads and applies.
- **Build entity state deltas** — ClusterServer builds the delta from simulation state; ReplicationChannelManager only passes it to channels.
- **Simulate or write to SpacetimeDB** — That is ClusterServer. ReplicationChannelManager only manages replication transport.
- **Authenticate or encrypt replication traffic** — Assumed private network (VPC). See IF-03.

---

## 4. Interface / Public API

ReplicationChannelManager is used in-process by ClusterServer. It does not expose a network API.

### 4.1 Lifecycle

```
start(cluster_id: ID, spacetimedb_client: Client) -> Result
```

Start the manager: subscribe to SpacetimeDB cluster_topology for this cluster_id (and optionally for neighbor clusters’ geometry). Begin applying topology updates. Does not open channels until topology subscription delivers at least one row with neighbor_ids.

```
stop() -> void
```

Close all channels with reason SHUTDOWN. Unsubscribe from SpacetimeDB. Called when ClusterServer is shutting down.

### 4.2 Send (used by ClusterServer)

```
send_to_neighbors(delta: EntityStateDelta) -> void
```

Broadcast the given delta to all current neighbor channels. Non-blocking; each channel enqueues (IReplicationChannel.send). Delta must include source_cluster_id (this cluster) and seq. EntityStateDelta shape is defined in IF-03.

Alternatively, the API can expose `get_channels() -> [IReplicationChannel]` and ClusterServer calls `send(delta)` on each channel. The doc assumes a single entry point `send_to_neighbors` for clarity; implementation may delegate to channels internally.

### 4.3 Receive (callback to ClusterServer)

```
on_receive(source_cluster_id: ID, delta: EntityStateDelta) -> void
```

Called by ReplicationChannelManager when an inbound delta is decoded from a subscription (Redis or TCP). ClusterServer implements this or registers a callback; it merges the delta into its neighbor-entity cache and performs gap detection (IF-03). ReplicationChannelManager is responsible for decoding and dispatching; it does not interpret entity state.

### 4.4 Destination geometry (for filtering)

```
set_neighbor_geometry(neighbor_cluster_id: ID, centroid: (x,y,z), spread_radius: float) -> void
```

Update the geometry used for observation-radius filtering when sending to that neighbor. Called when topology subscription delivers new or updated neighbor data. If cluster_topology does not store centroid/spread, ClusterManager or another component must push this via another path (e.g. a table or reducer); otherwise use defaults (e.g. large radius) so we do not over-filter.

### 4.5 Status (optional)

```
get_channel_count() -> int
get_channel_statuses() -> [ChannelStatus]
```

For metrics and debugging. ChannelStatus can include source_cluster_id, dest_cluster_id, connected, latency_ms, drop_count (from IF-03 get_status()).

---

## 5. Internal Structure

- **Topology subscription handler:** On SpacetimeDB subscription callback for cluster_topology: diff current neighbor_ids with previous set. For each new neighbor_id: resolve endpoint (from topology row or neighbor table — server_host, server_port for TCP; topic name = f("cluster:{cluster_id}") or similar for Redis). Create IReplicationChannel implementation instance (e.g. RedisPubSubReplication), call open(my_cluster_id, destination, config). Store in map neighbor_id → channel. For each removed neighbor_id: get channel, call close(NEIGHBOR_DEPARTED or CLUSTERS_MERGED), remove from map. If topology includes centroid/spread per cluster, call set_neighbor_geometry for each neighbor.
- **Send path:** send_to_neighbors(delta) iterates over the channel map and calls channel.send(delta) for each. Each channel may filter the delta (observation radius) using the destination geometry stored for that channel; IF-03 specifies the filter. Channels are responsible for encoding and publishing to Redis (or TCP).
- **Receive path:** Each IReplicationChannel has a subscription callback (Redis message handler or TCP read loop). On message: decode payload to EntityStateDelta, call on_receive(source_cluster_id, delta). source_cluster_id is the topic owner (Redis) or the connection’s cluster_id (TCP). ReplicationChannelManager does not buffer large backlogs; decoding and callback should be fast so the subscription thread is not blocked.
- **Concurrency:** Topology updates may arrive on a SpacetimeDB subscription thread; channel open/close and send may be called from ClusterServer’s tick thread. Access to the channel set must be thread-safe (e.g. mutex or concurrent map). Send can take a snapshot of the channel list to avoid holding the lock during send.

---

## 6. Data Ownership

- **Owns:** Set of IReplicationChannel instances; map of neighbor_cluster_id → channel; cached neighbor geometry (centroid, spread_radius) per neighbor; topology subscription handle.
- **Reads:** SpacetimeDB (cluster_topology subscription only). Receives decoded deltas from IReplicationChannel implementations (subscription callbacks).
- **Writes:** Nothing to SpacetimeDB or shared storage. Only in-process state. Outbound deltas are written to Redis (or TCP) by the IReplicationChannel implementation, not by ReplicationChannelManager directly.

---

## 7. Dependencies

| Dependency | What is used | If it changes |
|------------|--------------|----------------|
| SpacetimeDB | cluster_topology subscription (my cluster_id row; optionally neighbor rows for endpoint and geometry) | Schema and subscription query must match; neighbor_ids and endpoint info shape. |
| IReplicationChannel (IF-03) | open(), close(), send(), subscription callback | ReplicationChannelManager creates and holds channels; IF-03 defines lifecycle and delta shape. |
| ClusterServer (IN-02) | Calls send_to_neighbors(delta); implements or registers on_receive(source_cluster_id, delta) | ClusterServer must build delta with correct source_cluster_id and seq; handle gap detection on receive. |

ReplicationChannelManager does not depend on ClusterManager directly; it only reads from SpacetimeDB, which ClusterManager writes.

---

## 8. Message Protocol

ReplicationChannelManager does not define a client-facing message protocol. Internally it deals with:

- **SpacetimeDB:** Subscription to cluster_topology. Message shape is table row(s): cluster_id, neighbor_ids, server_host, server_port, optional centroid/spread columns.
- **Redis (or TCP):** IReplicationChannel implementation encodes EntityStateDelta (e.g. msgpack) and publishes to a topic. Topic naming: e.g. `arcane:replication:{cluster_id}` or as specified in wire/Redis doc. ReplicationChannelManager does not define the wire format; IF-03 and the implementation do.

---

## 9. Configuration

| Key | Default | Description |
|-----|---------|-------------|
| REPLICATION_OBSERVATION_RADIUS | 200.0 | Passed to IReplicationChannel config; entities beyond this from dest centroid are not sent (IF-03). |
| REPLICATION_MAX_QUEUE_DEPTH | 100 | Per-channel queue; see IF-03. |
| REPLICATION_SEND_INTERVAL_MS | 50 | Per-channel flush interval; see IF-03. |
| REPLICATION_COMPRESSION | true | Per-channel; see IF-03. |
| REDIS_URI | — | For RedisPubSubReplication; connection string. |
| (TCP implementation) | — | REPLICATION_PORT_OFFSET, destination host/port from topology. |

---

## 10. Metrics

| Metric | Type | Labels | Measures |
|--------|------|--------|----------|
| arcane_replication_channel_manager_subscription_count | gauge | cluster_id= | Number of active IReplicationChannel instances (neighbors we subscribe to). |
| arcane_replication_channel_manager_topology_update_total | counter | cluster_id= | Number of topology subscription updates applied. |
| arcane_replication_channel_manager_channel_open_total | counter | cluster_id= | Channels opened (new neighbor). |
| arcane_replication_channel_manager_channel_close_total | counter | cluster_id= | Channels closed (neighbor removed or shutdown). |

Per-channel metrics (send rate, drops, latency) are defined in IF-03 and exposed by the IReplicationChannel implementation; ReplicationChannelManager may re-export or aggregate them with cluster_id.

---

## 11. Failure Modes

| Failure | Detection | Response |
|---------|-----------|----------|
| SpacetimeDB topology subscription fails or disconnects | Subscription error | Retry subscribe. Until restored, do not add new channels; existing channels keep running. Optionally close all channels and set neighbor set to empty so we do not send to stale neighbors (conservative). |
| Topology row missing for my cluster_id | No row after startup or row removed | No neighbors; close all channels. Cluster may have been released (merge); ClusterServer may be shutting down or idle. |
| Redis (or broker) unreachable | IReplicationChannel reports disconnected | Channels handle reconnect (IF-03). ReplicationChannelManager does not remove the channel; when broker is back, channel resubscribes. |
| Channel open() fails (e.g. Redis subscribe fails) | open() returns Error | Log; do not add channel. Retry on next topology update or periodic retry. Emit metric. |

---

## 12. Open Questions

- **Topic naming:** Exact Redis topic format (e.g. `arcane:replication:{cluster_id}`) to be defined in a wire/Redis doc. ReplicationChannelManager must use the same convention when opening channels (subscribe to neighbor’s topic; our topic is where we publish).
- **Neighbor geometry source:** If cluster_topology does not store centroid/spread per cluster, how does ReplicationChannelManager get destination geometry for filtering? Options: ClusterManager writes it to topology; a separate table or view; or default to “send all” (no filter) until we add the columns.
- **Ordering of close vs. ClusterManager release:** When ClusterManager merges B into A and releases B’s server, it should update topology first (remove B from all neighbor lists), so all ReplicationChannelManagers close the channel to B before ClusterManager calls IServerPool.release(B). So no ordering issue if topology is updated before release.

---

*Arcane Engine — IN-06 ReplicationChannelManager — Confidential*
