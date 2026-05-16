# IF-03 — IReplicationChannel
**Cluster-to-Cluster State Broadcast Interface (Pub/Sub)**

---

| | |
|---|---|
| **Component ID** | IF-03 |
| **Layer** | Infrastructure Interface |
| **Type** | Interface — no implementation, only contract |
| **Purpose** | Define the contract for state broadcast between neighboring Arcane Nodes via **pub/sub**. Clusters **publish** their entity state and **subscribe** to neighbors' updates; there are no direct cluster-to-cluster connections. Allows the replication transport (e.g. Redis pub/sub) to be substituted without touching the ArcaneNode or ReplicationChannelManager. |
| **Implementations** | RedisPubSubReplication (default) · TCPReplicationChannel (alternative) · InProcessReplicationChannel (testing) |
| **Language** | Rust |
| **Depends On** | None |
| **Required By** | IN-02 ArcaneNode · IN-06 ReplicationChannelManager |

---

## 1. Overview

Replication between clusters uses **pub/sub**: each cluster **publishes** its entity state to a topic (e.g. keyed by cluster_id) and **subscribes** to the topics of its neighbors. There are no direct cluster-to-cluster connections. IReplicationChannel represents the contract for this: one cluster's subscription to another's updates (and the corresponding publish path). Each Arcane Node maintains a set of IReplicationChannel instances (one per current neighbor) through the ReplicationChannelManager — each instance represents "we subscribe to that neighbor's topic and publish to our own."

The interface is deliberately fire-and-forget on the publish side. Replication data is ephemeral — a position update that fails to arrive is superseded by the next one 50ms later. Pub/sub does not guarantee delivery and we do not implement acknowledgement or retry. It optimizes for throughput and low latency, not reliability. Reliability is the job of the game state layer (SpacetimeDB), not the replication layer.

Replication carries **simulation state** (position, movement) only. **Discrete game outcomes** (damage applied, ability resolved) are handled by **SpacetimeDB reducers**; the ArcaneNode calls the reducer and sends RPC_RESULT to the client from the reducer return. Optional RPCHandler (IN-05) is for non-game server-to-server RPC only.

---

## 2. Responsibilities

- Subscribe to one neighboring cluster's state topic (and publish this cluster's state to a topic that neighbor subscribes to)
- Accept entity state deltas and publish them with minimal latency; receive deltas from subscriptions
- Report subscription/transport health (connected, latency, drop rate) to the ReplicationChannelManager
- Reconnect or resubscribe automatically on broker or subscription failure without intervention from the ClusterManager
- Apply observation-radius filtering — only publish entities within the configured radius of the destination cluster's centroid (centroid + spread_radius + observation_radius)

---

## 3. What It Does NOT Do

- Guarantee message delivery — fire and forget
- Implement acknowledgement or retry — not used for game logic; game outcomes use SpacetimeDB reducers
- Make decisions about which clusters are neighbors (who subscribes to whom) — that is ReplicationChannelManager's job
- Buffer large backlogs — if the destination is slow, drop messages rather than accumulate memory
- Encrypt or authenticate traffic — assumed to be on a private VPC or equivalent trusted network

---

## 4. Interface Definition

### 4.1 Lifecycle

```
open(source_cluster_id: UUID, destination: ServerHandle, config: ChannelConfig) -> Result
```

Establishes the replication relationship with the destination cluster: subscribe to that cluster's state topic and (for the reverse direction) ensure this cluster's topic is available for the destination to subscribe to. With Redis pub/sub this means subscribing to the destination's topic; no direct connection to the destination server. Returns immediately after the subscription is active — does not block on any game state.

```
ChannelConfig {
  observation_radius:   float    // only send entities within this radius of dest centroid
  max_queue_depth:      int      // drop oldest messages if queue exceeds this (default 100)
  send_interval_ms:     int      // flush interval in ms (default 50 — matches tick rate)
  compression_enabled:  bool     // msgpack compression for large payloads (default true)
}
```

```
close(reason: CloseReason) -> void
```

Stops subscribing to the destination cluster's topic and flushes any pending publish queue. Called by ReplicationChannelManager when two clusters are no longer neighbors.

```
CloseReason { NEIGHBOR_DEPARTED | CLUSTERS_MERGED | SHUTDOWN }
```

---

### 4.2 Send

```
send(delta: EntityStateDelta) -> void
```

Enqueues an entity state delta for transmission. Non-blocking. If the queue is full (max_queue_depth exceeded), the oldest item is dropped and `arcane_replication_drops_total` is incremented.

```
EntityStateDelta {
  source_cluster_id:  UUID
  seq:                int       // incremental per source_cluster_id — receiver uses this to detect gaps
  tick:               int
  timestamp:          float
  
  updated: [EntityStateEntry]   // entities with changed state since last tick
  removed: UUID[]               // entity_ids that left this cluster
}

EntityStateEntry {
  entity_id:    UUID
  cluster_id:   UUID
  position:     Vector3      // bucket 1 — spine
  velocity:     Vector3      // bucket 1 — spine
  user_data:    JSON | null  // bucket 2 — on wire when present (game-defined schema)
  // local_data: JSON — bucket 3; not serialized on EntityStateDelta (cluster process only)
}
```

See `docs/architecture/four-bucket-state-model.md` for the full model. Game-specific fields (health, anim, etc.) typically live in **`user_data`** (replicated) or SpacetimeDB (**bucket 4**) until finer-grained wire types exist.

---

### 4.3 Status

```
get_status() -> ChannelStatus
```

```
ChannelStatus {
  source_cluster_id:   UUID
  dest_cluster_id:     UUID
  connected:           bool
  latency_p99_ms:      float
  send_rate_hz:        float      // actual sends per second
  drop_rate_pct:       float      // messages dropped due to full queue
  bytes_per_second:    float
  last_send_at:        float
}
```

### 4.4 Gap detection and recovery

Replication is fire-and-forget; messages can be dropped (e.g. pub/sub at-most-once). To recover from missing messages (including lost removals), each delta carries an incremental sequence id (`seq`) per source cluster so the receiver can detect gaps.

When the destination detects a gap (missing message(s)), it does not need to identify which source cluster had the gap. It re-reads current state from SpacetimeDB for the state it cares about. That full sync restores consistency — including removals, since entities not present in the snapshot are absent. Recovery is always: gap detected → read current state from SpacetimeDB.

---

## 5. Observation Radius Filtering

**Neighbor definition:** Which clusters subscribe to which is determined by observation radius (and centroid+spread): two clusters are neighbors (and subscribe to each other's updates) if they could have players or entities within observation range of each other — i.e. centroid + spread_radius + observation_radius defines the effective area; clusters whose effective areas can overlap or come within range are neighbors. Merge/split is a separate optimization (graph+ML, topology). Replication uses the same observation-radius and centroid+spread for both "who subscribes to whom" and "what to send," so we never miss entities that a player could see.

**What we send:** Before enqueuing an entity for transmission, the publish path checks whether the entity is within range of the destination cluster using a **centroid + spread_radius** model. This is more accurate than centroid-only filtering while remaining cheap to compute. A cluster that is tightly packed has a small spread_radius, so the effective radius is smaller; a cluster where players are spread out has a larger spread_radius, so we send entities in a larger area and do not miss ones that a distant player could see.

```
effective_radius = observation_radius + dest_cluster.spread_radius

distance(entity.position, dest_centroid) <= effective_radius
```

Where `spread_radius` is the maximum distance of any player in the destination cluster from its centroid. It is maintained incrementally by the destination ArcaneNode — updated on every player position change at the same cadence as the centroid itself. One additional float per cluster, one addition per entity check on the hot path.

```rust
fn should_send(entity: &EntityStateEntry, dest: &DestClusterState) -> bool {
    let effective_radius = self.config.observation_radius + dest.spread_radius;
    entity.position.distance(dest.centroid) <= effective_radius
}
```

The destination centroid and spread_radius are pushed by the ReplicationChannelManager each time the ClusterManager updates neighbor topology (who subscribes to whom). Not fetched per-send.

### Tradeoff: False Positives Are Acceptable, False Negatives Are Not

The centroid+radius model may over-include entities in directions where the destination cluster happens to be spread out but has no players nearby. These false positives cost a few extra bytes of bandwidth. The alternative — missing an entity that a destination player can actually see — produces a visible pop-in artifact. The tradeoff strongly favors over-inclusion.

> **Future optimization — Two-phase broad/narrow filter:** For extreme convergence scenarios (hundreds of entities converging on a point across multiple clusters), a two-phase filter can reduce false positive bandwidth waste. Phase 1: centroid+radius coarse check (cheap). Phase 2: if triggered, check distance against each individual destination player position (precise). This is documented as a named optimization path to consider if bandwidth profiling during benchmark shows false positive overhead is material at maximum player density. Do not implement unless the benchmark demonstrates a real problem.

---

## 6. Update Frequency Tiering

Entities are transmitted at different frequencies based on their distance from the destination centroid. Entities closer to the boundary between clusters update more frequently than entities deep in the source cluster's territory.

| Distance from dest centroid | Update frequency |
|---|---|
| 0 — 40 units (near boundary) | Every tick — 20Hz |
| 40 — 100 units | Every 4th tick — 5Hz |
| 100 — observation_radius | Every 20th tick — 1Hz |
| Beyond observation_radius | Not sent |

The publish path tracks last-sent tick per entity and skips entities that are not due for an update this tick.

---

## 7. Internal Structure

### RedisPubSubReplication (default implementation)

Replication transport is Redis pub/sub. No direct cluster-to-cluster connections.

```
# Startup
open():
  subscribe_to(redis, topic_for(destination.cluster_id))   // subscribe to neighbor's state topic
  start receive_loop()   // handle incoming messages from subscription
  // This cluster's state is published to topic_for(source_cluster_id); neighbors subscribe to it

# Publish loop (per tick or send_interval_ms)
  batch = drain_queue()
  if batch is not empty:
    payload = msgpack.encode(batch)
    if compression_enabled: payload = zlib.compress(payload)
    redis.publish(topic_for(source_cluster_id), payload)
  update_metrics()

# Receive side: subscription callback delivers messages to ReplicationChannelManager.on_receive()
```

close(): Unsubscribe from destination's topic. Flush pending publish queue.

### TCPReplicationChannel (alternative implementation)

For environments where Redis is not used, a direct TCP connection to the destination Arcane Node is an alternative. open() connects to destination.host:rpc_port+1; send_loop() sends batches over the socket. Same interface, different transport.

### InProcessReplicationChannel (testing)

For unit and integration tests, this implementation calls the destination's `on_receive()` method directly in the same process with no network hop. Allows full end-to-end replication testing without running multiple processes.

---

## 8. Data Ownership

- **Owns:** Publish queue, subscription state (or connection state for TCP implementation), per-entity last-sent tick tracking
- **Reads:** Destination cluster centroid (pushed by ReplicationChannelManager), entity state deltas (provided by ArcaneNode per tick)
- **Writes:** Nothing to shared storage — all state is in-process

---

## 9. Dependencies

None at interface level. RedisPubSubReplication depends on Redis client availability. TCPReplicationChannel depends on TCP socket availability. Both implementations are self-contained.

---

## 10. Configuration

| Key | Default | Description |
|---|---|---|
| `REPLICATION_OBSERVATION_RADIUS` | `200.0` | Units — entities beyond this from dest centroid are not sent |
| `REPLICATION_MAX_QUEUE_DEPTH` | `100` | Messages before oldest is dropped |
| `REPLICATION_SEND_INTERVAL_MS` | `50` | Flush interval — matches simulation tick rate |
| `REPLICATION_COMPRESSION` | `true` | Enable zlib compression on payloads |
| `REPLICATION_PORT_OFFSET` | `1` | (TCP implementation only) Port = rpc_port + offset for replication listener |
| `REPLICATION_RECONNECT_INTERVAL_MS` | `500` | Delay between reconnect or resubscribe attempts on broker/subscription loss |

---

## 11. Metrics

| Metric | Type | Labels | Measures |
|---|---|---|---|
| `arcane_replication_send_rate_hz` | gauge | `source=, dest=` | Publishes per second per subscription |
| `arcane_replication_bytes_per_second` | gauge | `source=, dest=` | Bandwidth per subscription |
| `arcane_replication_latency_ms` | histogram | `source=, dest=` | Round-trip latency (measured via heartbeat) |
| `arcane_replication_drops_total` | counter | `source=, dest=` | Messages dropped due to full queue |
| `arcane_replication_entities_per_delta` | histogram | | Entities transmitted per tick per subscription |
| `arcane_replication_filtered_pct` | gauge | `source=, dest=` | Fraction of entities filtered by observation radius |
| `arcane_replication_subscription_count` | gauge | `cluster_id=` | Active subscriptions (neighbors we subscribe to) per Arcane Node |
| `arcane_replication_reconnects_total` | counter | `source=, dest=` | Reconnect or resubscribe events |

---

## 12. Failure Modes

| Failure | Detection | Response |
|---|---|---|
| Pub/sub broker unreachable or subscription fails | Redis connection lost or subscribe fails | Mark subscription inactive. Attempt resubscribe every REPLICATION_RECONNECT_INTERVAL_MS. Drop all deltas while disconnected — do not buffer. Notify ReplicationChannelManager. (TCP implementation: destination server unreachable.) |
| Queue full | Queue depth > max_queue_depth | Drop oldest item. Increment drops counter. Log warning if drop rate > 5% sustained. |
| Destination cluster merged away | ReplicationChannelManager calls close() | Flush publish queue. Unsubscribe. No further action. |
| High latency (p99 > 30ms) | Heartbeat or round-trip measurement | Emit metric. Log warning. Do not unsubscribe — high latency replication is still better than no replication. |
| Sustained drop rate > 20% | metrics check | ReplicationChannelManager may signal ClusterManager to consider merging the two clusters — high drop rate indicates heavy interaction that may warrant co-location. |

---

## 13. Open Questions

- **Compression threshold:** Compression adds CPU cost. For small deltas the compression overhead may exceed the bandwidth saving. A minimum payload size threshold below which compression is skipped should be tuned empirically against benchmark data.

---

*Arcane Engine — IF-03 IReplicationChannel — Confidential*
