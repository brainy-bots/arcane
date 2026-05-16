# IN-02 — ArcaneNode
**Player simulation unit**

---

| | |
|---|---|
| **Component ID** | IN-02 |
| **Layer** | Infrastructure |
| **Type** | Component |
| **Purpose** | One process per cluster. Runs **high-frequency simulation** (movement, physics, **AI behaviors**) for entities it owns; accepts client Cluster connections (WebSocket); sends STATE_UPDATE and RPC_RESULT; receives PLAYER_INPUT and HANDOFF_CLAIM. Subscribes to SpacetimeDB for assignments and topology; writes entity state to SpacetimeDB at a **throttled rate**; publishes state to Redis via IReplicationChannel and receives neighbors' state. For **discrete game events** (e.g. attack hit), calls SpacetimeDB reducers and sends RPC_RESULT to the client from the reducer return. Optionally hosts RPCHandler (IN-05) for non-game server-to-server RPC. |
| **Document version** | 1.0 |

---

## 1. Overview

ArcaneNode is the process that clients connect to for a given cluster. It owns a set of players and entities (defined by SpacetimeDB assignments); it runs **high-frequency simulation** (movement, physics, **AI**) each tick, writes entity state to SpacetimeDB at a **throttled rate** (not every tick), and publishes state deltas to Redis via IReplicationChannel so neighbors see movement in real time. For **discrete game events** (e.g. projectile hits target), it **calls a SpacetimeDB reducer**; the reducer updates game tables and returns (success, state_tick); the server sends **RPC_RESULT** to the client from that return. It subscribes to neighbors' replication topics and to SpacetimeDB for assignments and gap recovery. It does not decide assignments or topology — ArcaneManager does. See `00_component_index.md` § Simulation vs authoritative world state; `docs/END_TO_END_FLOWS.md`, `in_01_manager.md`, `docs/SPACETIMEDB_SCHEMA.md`, `ca_01_iclientadapter.md`.

---

## 2. Responsibilities

- **Obtain cluster identity at startup:** Learn `cluster_id` (and optionally `server_id`) from environment, orchestration, or by subscribing to `cluster_topology` where this process is the server (e.g. by host:port or server_id). Until ArcaneManager creates a row in `cluster_topology` for this server, the ArcaneNode may have no cluster_id (idle server in pool) or may be started on demand with a pre-assigned cluster_id — see Configuration and Open Questions.
- **Subscribe to SpacetimeDB:** For `cluster_assignments` and `entity_assignments` filtered by `cluster_id = my_cluster_id`; for `entity_state` filtered by `cluster_id = my_cluster_id` (own entities). Optionally subscribe to `cluster_topology` for my cluster_id to get `neighbor_ids` and endpoint info (or ReplicationChannelManager does this). Subscription callbacks update the in-memory list of "my players" and "my entities."
- **Run simulation tick:** Each tick (e.g. 20 Hz), run **simulation** (physics, movement, **AI behaviors**) for all entities this server owns. Apply player input (from PLAYER_INPUT) to player entities; run AI for NPCs/monsters. Produce updated entity state. **Do not** run discrete game logic (e.g. "apply damage") in the tick — that is done via SpacetimeDB reducers when an event occurs (e.g. projectile hit).
- **Write owned entity state to SpacetimeDB:** At a **throttled rate** (e.g. every 1–2 s per entity or on significant change), call `upsert_entity_state` for owned entities. Only write for entities still assigned to this cluster_id. High-frequency position updates are replicated via Redis, not written every tick.
- **Publish state to replication:** After tick, send entity state deltas for owned entities (within observation-radius filtering) to the replication layer via IReplicationChannel. ReplicationChannelManager holds the set of IReplicationChannel instances (one per neighbor); ArcaneNode (or a sub-component) calls `send(delta)` on each. Deltas include `seq` for gap detection.
- **Receive neighbor state from replication:** ReplicationChannelManager delivers incoming deltas from neighbors' topics. On receive: merge into local view of "neighbor entities." On gap (missing `seq`): trigger full sync from SpacetimeDB for affected state (see IF-03 § Gap detection and recovery).
- **Accept client Cluster connections:** Listen on the Cluster WebSocket port (e.g. 8080 + n). Accept connections; associate each connection with a player_id (from first message, e.g. "I am player P" or from HANDOFF_CLAIM with handoff_token). Only accept players that are in `cluster_assignments` for this cluster_id (learned from SpacetimeDB).
- **Send STATE_UPDATE to connected clients:** Each tick, send STATE_UPDATE (delta or full) over each client's WebSocket with the visible entity set (own entities + entities received from replication). Include `tick`, `seq`, `updated`, `removed_entity_ids`. Use delta by default; full sync on client request or after gap recovery.
- **Receive PLAYER_INPUT from clients:** Deserialize and apply to the corresponding player entity in the simulation. Batch by tick; process in tick order.
- **Handle HANDOFF_CLAIM:** When a client sends HANDOFF_CLAIM (handoff_token, player_id), validate token (e.g. against ArcaneManager-issued tokens or accept if player_id is in my cluster_assignments). Reply with HANDOFF_ACCEPTED. Associate the connection with that player.
- **Discrete game events → SpacetimeDB reducers:** When a discrete event occurs (e.g. projectile hits target, player uses item), **call the appropriate SpacetimeDB reducer** (e.g. `attack_hit`). The reducer updates game tables and returns (success, state_tick). Send **RPC_RESULT** to the client from that return (request_id, result, state_tick). No TCP RPC between cluster servers for game logic — cross-cluster coordination is inside SpacetimeDB.
- **Optional: host RPCHandler (IN-05):** If present, expose TCP port for **non-game** server-to-server RPC (admin, tools). Game actions do not use this path.
- **Expose Prometheus metrics:** Tick rate, connected client count, entity count, replication send/recv rates, RPC latency, SpacetimeDB write rate.

---

## 3. What It Does NOT Do

- **Assign players or entities to clusters** — ArcaneManager writes assignments; ArcaneNode only reads.
- **Decide merge/split or topology** — ArcaneManager and IClusteringModel do that; ArcaneNode reacts to subscription updates (drop entities no longer mine, ReplicationChannelManager opens/closes subscriptions from topology).
- **Authenticate players** — Auth may be done at Manager connection or at first Cluster message; ArcaneNode trusts that ArcaneManager only assigned valid players to this cluster (or validates token if provided).
- **Guarantee replication delivery** — Replication is fire-and-forget; gap recovery is via SpacetimeDB full sync.
- **Run ArcaneManager or ReplicationChannelManager** — Those are separate processes or components; ArcaneNode uses ReplicationChannelManager (and IReplicationChannel) for publish/subscribe.

---

## 4. Interface / Public API

ArcaneNode is a long-running process. It does not expose a public API to other services; it exposes:
- **Cluster WebSocket** (clients): accept connections, send STATE_UPDATE / HANDOFF_ACCEPTED / RPC_RESULT, receive PLAYER_INPUT / HANDOFF_CLAIM.
- **Optional RPC TCP port** (IN-05 RPCHandler): for non-game server-to-server RPC only. Game logic uses SpacetimeDB reducers; RPC_RESULT to the client comes from reducer return.

Internal interfaces it uses:
- **SpacetimeDB client:** Subscribe to cluster_assignments, entity_assignments, entity_state (filtered by cluster_id); call reducers upsert_entity_state, delete_entity_state.
- **ReplicationChannelManager:** Get IReplicationChannel instances for each neighbor; call send(delta) on each; receive callbacks or stream of deltas from subscriptions. ReplicationChannelManager subscribes to cluster_topology and opens/closes channels — ArcaneNode only feeds data and consumes received data.
- **IWorldSimulator (optional):** For unobserved or low-priority entities, call FastForward or equivalent (see IF-04). ArcaneNode may delegate to a component that implements IWorldSimulator for entities not currently observed by any player.

---

## 5. Internal Structure

- **Tick loop:** Fixed interval (e.g. 20 Hz). Each tick: (1) Drain PLAYER_INPUT and apply to player entities. (2) Run **simulation** for owned entities (physics, movement, AI). (3) Remove entities that left this cluster at end of tick. (4) **Throttled:** call SpacetimeDB `upsert_entity_state` only when due (e.g. every 1–2 s per entity). (5) Build EntityStateDelta for owned entities and call IReplicationChannel.send(delta). (6) On **discrete event** (e.g. hit): call SpacetimeDB reducer; send RPC_RESULT to client from reducer return. (7) Build STATE_UPDATE (own + replicated neighbor entities) and send to clients. (8) Increment tick and seq.
- **SpacetimeDB subscription handlers:** On new row in cluster_assignments or entity_assignments for my cluster_id: add player/entity to "mine." On removed or updated row (cluster_id changed): remove or update at end of current tick. On entity_state update for my cluster_id: update local cache (for gap recovery or initial load).
- **Replication receive path:** When a delta arrives from a neighbor (via IReplicationChannel callback or queue): merge into "neighbor entity" cache. If seq gap detected: request full state from SpacetimeDB for that source cluster or for visible set, then replace cache. Include neighbor entities in STATE_UPDATE to clients.
- **Client connection state:** Map connection_id → player_id (once known). Map player_id → connection_id for sending STATE_UPDATE. On HANDOFF_CLAIM or first message with player_id, fill the mapping. On disconnect, remove; do not remove from cluster_assignments (ArcaneManager handles leave).
- **Discrete-event path:** When simulation detects a discrete event (e.g. projectile hits target), the server **calls the game's SpacetimeDB reducer** (e.g. `attack_hit`) with the relevant IDs and params. The reducer updates game tables and returns (success, state_tick). The server sends **RPC_RESULT** (request_id, result, state_tick) to the client over the Cluster WebSocket. Client uses state_tick for prediction/reconciliation (CA-01). No cross-cluster TCP RPC for game logic.

---

## 6. Data Ownership

- **Owns:** In-memory simulation state for owned entities; client connection map (connection_id ↔ player_id); local "neighbor entity" cache (from replication); tick counter and seq counter for replication.
- **Reads:** SpacetimeDB (subscriptions): cluster_assignments, entity_assignments, entity_state (filtered by cluster_id). Receives from IReplicationChannel (neighbor deltas). Receives PLAYER_INPUT and HANDOFF_CLAIM from clients.
- **Writes:** SpacetimeDB (upsert_entity_state, delete_entity_state for owned entities only). Sends to clients: STATE_UPDATE, HANDOFF_ACCEPTED, RPC_RESULT. Sends to replication: EntityStateDelta via IReplicationChannel.send().

---

## 7. Dependencies

| Dependency | What is used | If it changes |
|------------|--------------|----------------|
| SpacetimeDB | Subscriptions (assignments, entity_state), reducers (upsert_entity_state, delete_entity_state; game reducers for discrete events) | Schema and reducer names must match; gap recovery assumes we can read full state; RPC_RESULT comes from reducer return. |
| ReplicationChannelManager (IN-06) | Neighbor list and IReplicationChannel instances; send(delta), receive deltas | ArcaneNode must still produce and consume deltas with seq; IN-06 handles who to subscribe to. |
| IReplicationChannel (IF-03) | send(EntityStateDelta), receive path (callback or stream) | Delta shape and seq semantics must match; gap recovery behavior is specified in IF-03. |
| RPCHandler (IN-05) | Optional. TCP endpoint for non-game RPC only | If used, ArcaneNode hosts it; game logic does not flow through it. |
| IWorldSimulator (IF-04) | Optional: FastForward for unobserved entities | If used, ArcaneNode or a sub-component calls it; IF-04 defines the contract. |

---

## 8. Message Protocol

### 8.1 Cluster WebSocket (ArcaneNode ↔ client)

**Client → ArcaneNode:**

| Message | Format | When |
|---------|--------|------|
| PLAYER_INPUT | `{ type, player_id, position, velocity, action, action_data?, timestamp, sequence_num, last_state_seq }` | Every tick (e.g. 20 Hz). |
| HANDOFF_CLAIM | `{ type, handoff_token, player_id }` | On reconnect after CLUSTER_REASSIGN (merge/split). |

**ArcaneNode → Client:**

| Message | Format | When |
|---------|--------|------|
| STATE_UPDATE | `{ type, tick, timestamp, seq, updated: [EntityStateDelta], removed_entity_ids }` | Every tick; delta of visible entities. |
| STATE_UPDATE_FULL | `{ type, tick, timestamp, seq, entities: [EntitySnapshot], removed_entity_ids }` | On request or after gap recovery. |
| HANDOFF_ACCEPTED | `{ type, cluster_id, player_count }` | After valid HANDOFF_CLAIM. |
| RPC_RESULT | `{ type, request_id, result: SUCCESS \| FAILURE, state_tick }` | After RPC completes (cross-cluster or local). |

See CA-01 §7 for full field definitions and client-side prediction semantics (state_tick).

### 8.2 First connection (no handoff)

On first join, the client connects without sending HANDOFF_CLAIM. The server must identify the client: e.g. client sends an initial message with player_id (and optionally auth_token), or the server infers from a token. The server checks that player_id is in cluster_assignments for this cluster_id (from SpacetimeDB). If valid, associate connection with player_id and start STATE_UPDATE stream. TBD in wire protocol doc (see in_01_manager.md Open Questions).

---

## 9. Configuration

| Key | Default | Description |
|-----|---------|--------------|
| NODE_HOST | 0.0.0.0 | Bind address for Cluster WebSocket. |
| NODE_PORT | 8080 | Base port; may be 8080 + n when multiple servers on same host. |
| CLUSTER_ID | — | This server's cluster_id (if set at startup). Else learned from cluster_topology (e.g. by SERVER_ID). |
| SERVER_ID | — | Opaque server id from pool; used to find cluster_id in cluster_topology. |
| TICK_RATE_HZ | 20 | Simulation and STATE_UPDATE rate. |
| SPACETIMEDB_URI | — | SpacetimeDB connection URI. |
| (ReplicationChannelManager / Redis config) | — | See IN-06, IF-03. |
| (RPCHandler port) | 9200 + n | See IN-05. |

---

## 10. Metrics

| Metric | Type | Labels | Measures |
|--------|------|--------|----------|
| arcane_node_tick_duration_ms | histogram | | Time per simulation tick. |
| arcane_node_tick_rate_hz | gauge | | Actual tick rate (1 / tick_interval). |
| arcane_node_connected_clients | gauge | cluster_id= | Number of open Cluster WebSocket connections. |
| arcane_node_owned_entity_count | gauge | cluster_id= | Number of entities assigned to this cluster (from subscription). |
| arcane_node_spacetime_writes_total | counter | cluster_id= | upsert_entity_state / delete_entity_state calls. |
| arcane_node_replication_sent_total | counter | cluster_id= | Deltas sent via IReplicationChannel.send(). |
| arcane_node_replication_received_total | counter | cluster_id= | Deltas received from neighbors. |
| arcane_node_replication_gap_recoveries_total | counter | cluster_id= | Gap detected and full sync from SpacetimeDB performed. |
| arcane_node_rpc_requests_total | counter | cluster_id= | Incoming RPCs (local + cross-cluster). |
| arcane_node_rpc_latency_ms | histogram | cluster_id= | RPC handling latency. |

---

## 11. Failure Modes

| Failure | Detection | Response |
|---------|-----------|----------|
| SpacetimeDB unreachable | Connection or subscription error | Retry with backoff. Continue simulation and replication from last known state; do not accept new HANDOFF_CLAIM until subscriptions restored. Gap recovery will fail until SpacetimeDB is back. |
| Subscription says entity no longer mine | cluster_assignments or entity_assignments update (cluster_id changed) | Drop entity at end of current tick; stop writing and publishing for it. Close client connections for players that left (merge/split); client will reconnect to new cluster. |
| Replication channel disconnected | IReplicationChannel or broker failure | ReplicationChannelManager reconnects (IF-03). Until then, neighbor state is stale; on reconnect, gap detection may trigger full sync. |
| Client disconnect | WebSocket close | Remove from connection map; stop sending STATE_UPDATE to that connection. Do not change cluster_assignments (ArcaneManager handles leave if needed). |
| Tick overrun | Tick duration > tick interval | Log; optionally skip next tick or run late. Emit metric. Clients may see lower effective STATE_UPDATE rate. |
| Invalid HANDOFF_CLAIM | Unknown or expired handoff_token, or player_id not in my cluster | Reject (close connection or send error). Client should request fresh assignment from Manager. |

---

## 12. Open Questions

- **Cluster identity at startup:** When ArcaneNode starts (e.g. from pool), does it get cluster_id from env (ArcaneManager passes it when allocating) or does it subscribe to cluster_topology and wait for a row where server_id = me? For pool of pre-started servers, likely server_id is known and cluster_id is created when ArcaneManager first assigns players to this server (then topology row appears). Idle server may have no cluster_id until first assignment.
- **First-connect client message:** Exact format and semantics of the first message from client to ArcaneNode on first join (no handoff) — player_id, auth_token, or both — to be defined in wire protocol doc.
- **Handoff token validation:** Where are handoff tokens issued (ArcaneManager) and how does ArcaneNode validate them (shared secret, SpacetimeDB table, or trust Manager)? TBD.

---

*Arcane Engine — IN-02 ArcaneNode — Confidential*
