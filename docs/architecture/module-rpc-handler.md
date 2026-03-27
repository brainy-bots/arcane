# IN-05 — RPCHandler
**Optional TCP endpoint for non-game server-to-server RPC**

---

| | |
|---|---|
| **Component ID** | IN-05 |
| **Layer** | Infrastructure |
| **Type** | Component (optional) |
| **Purpose** | **Optional.** TCP server on each ClusterServer for **non-game** server-to-server RPC (e.g. admin, tooling, health checks). **Game logic** (attack hit, inventory, spells) is **not** handled here — it is implemented as **SpacetimeDB reducers**; the client's ClusterServer calls the reducer and sends RPC_RESULT to the client from the reducer return. RPCHandler is for internal or operational RPCs only. |
| **Document version** | 1.0 |

---

## 1. Overview

**What RPCHandler is:** An **optional** TCP endpoint (port 9200 + n per server) for **non-game** request–response calls (e.g. admin, tooling, stats). **Not** for game actions (combat, inventory). For those: ClusterServer calls a SpacetimeDB reducer and sends RPC_RESULT to the client from the reducer return (IN-02, CA-01). Example (non-game): tooling asks cluster B for stats. B’s RPCHandler returns (request_id, result, payload). B’s RPCHandler returns (request_id, result, payload). No client RPC_RESULT flows through RPCHandler; game actions use SpacetimeDB reducers (IN-02, CA-01).

**What RPCHandler is not:** It does **not** talk to SpacetimeDB. SpacetimeDB is used for:
- **ClusterManager:** subscriptions (live view) and reducers (assignments, topology).
- **ClusterServer:** subscriptions (assignments, entity_state) and reducers (upsert_entity_state, delete_entity_state).

RPC is for **immediate, request–response game actions** that change state on the **target cluster’s simulation**; that state is then persisted by the ClusterServer’s normal tick (writes to SpacetimeDB). So: RPC → execute on cluster B → B’s simulation and SpacetimeDB writes are separate; the handler only triggers the local game logic and returns the outcome.

---

## 2. Responsibilities

- **Listen on the RPC TCP port** (e.g. 9200 + n) for connections from other ClusterServers (or the same server for local callers). Accept and parse RPC request messages (request_id, caller_cluster_id, source_player_id, target_entity_id, action type, action params).
- **Route each request to local game logic:** If `target_entity_id` is owned by this cluster (present in this server’s owned-entity set from SpacetimeDB assignments), invoke the appropriate game handler (e.g. combat, spell) and run the action in the current tick or next tick. If the target is not owned by this cluster, return an error (e.g. entity moved or invalid target).
- **Return a response** to the caller: (request_id, result: SUCCESS | FAILURE, state_tick). `state_tick` is the tick (or seq) of the STATE_UPDATE in which the action was applied, so the client can correlate RPC_RESULT with the authoritative state update (CA-01).
- **Not send to clients directly:** The **caller** ClusterServer (the one the client is connected to) is responsible for sending **RPC_RESULT** to the client over the Cluster WebSocket. RPCHandler only returns the result to that caller over TCP.
- **Expose metrics:** RPC request count, latency histogram, failure count (unknown target, validation failure, etc.), optionally per-action-type.

---

## 3. What It Does NOT Do

- **Communicate with SpacetimeDB** — No subscriptions, no reducers. SpacetimeDB is used by ClusterManager and by the ClusterServer’s main loop (subscribe + write entity state). RPCHandler only triggers in-process game logic.
- **Replicate entity state** — Replication (Redis pub/sub) is handled by ReplicationChannelManager and IReplicationChannel. RPC is request–response; replication is fire-and-forget state broadcast.
- **Decide which cluster owns an entity** — Ownership comes from SpacetimeDB (entity_assignments). The **caller** server uses that (or a local cache) to decide “target is in cluster B” and thus to send the RPC to B’s RPCHandler.
- **Send RPC_RESULT to the client** — The caller ClusterServer sends RPC_RESULT over the client’s WebSocket. RPCHandler only returns the result to the caller server.

---

## 4. Interface / Public API

RPCHandler is **optional**. When enabled, the ClusterServer process hosts it. It exposes:

- **Start:** Bind to the configured TCP port (e.g. 9200 + n), accept connections. No public “API” beyond the **wire protocol** below.
- **Integration with ClusterServer:** ClusterServer provides:
  - A way to **execute an action** on a local entity (e.g. `execute_rpc(target_entity_id, action, params) -> (success, state_tick)`). The handler calls this and returns the result to the TCP client.
  - Optionally, a way to **send an outbound RPC** to another cluster (caller side): given target_entity_id, resolve target cluster (from entity_assignments or cache), open TCP to that cluster’s RPC port, send request, wait for response, then send RPC_RESULT to the client. That “caller side” may live in ClusterServer or in a small RPC-client layer next to the handler; the handler itself is the **server** (receiver) side.

---

## 5. Internal Structure

- **Server side (this component):** Listener thread or async task accepts TCP connections. For each connection: read length-prefixed or framed messages; parse into (request_id, caller_cluster_id, source_player_id, target_entity_id, action, params). Look up target_entity_id in the set of entities owned by this cluster. If not found or invalid, send (request_id, FAILURE, 0). If found, call into ClusterServer’s game logic to execute the action; get back (success, state_tick). Send (request_id, success ? SUCCESS : FAILURE, state_tick) back over the same connection. Connection may be kept open for multiple RPCs (connection pool per cluster pair) or one-shot; design choice.
- **Caller side (ClusterServer or RPC client):** When the local server receives a client-initiated action whose target is on another cluster, it opens (or reuses) a TCP connection to that cluster’s RPC port, sends the RPC message, blocks or async-waits for the response, then sends RPC_RESULT to the client. RPCHandler doc focuses on the **receiver**; the caller behavior can be described in ClusterServer or in this doc under “Caller flow.”
- **Execution:** Action execution (e.g. deal damage, apply effect) is **game logic** (game layer or ClusterServer’s simulation). RPCHandler only dispatches to it and returns the result. It does not implement combat or spells.

---

## 6. Data Ownership

- **Owns:** TCP listener, connection state (open connections from other ClusterServers), request/response buffers. No ownership of entity state or SpacetimeDB.
- **Reads:** Only the request payload and the result from the local execution callback (success, state_tick). Entity ownership is implied by “target is in my cluster” (ClusterServer’s owned-entity set).
- **Writes:** Only the TCP response back to the caller. No writes to SpacetimeDB or Redis; game state changes are done by the simulation and written by the normal ClusterServer tick.

---

## 7. Dependencies

| Dependency | What is used | If it changes |
|------------|--------------|----------------|
| ClusterServer (IN-02) | Optionally hosts the handler; provides “execute action on local entity” and owned-entity set | Handler must match the non-game action API. |
No dependency on SpacetimeDB or game logic. RPC_RESULT for **game actions** is sent by ClusterServer from SpacetimeDB reducer return (CA-01); RPCHandler is not involved.

---

## 8. Message Protocol (Wire Format)

**Direction:** Caller (tooling or ClusterServer) → This ClusterServer (TCP, port 9200 + n). **Non-game only.**

**Request (caller → target):**

| Field | Type | Description |
|-------|------|-------------|
| request_id | ID | Unique per request; echoed in response. |
| caller_cluster_id | ID | Optional; for logging and metrics. |
| action_type | string or enum | e.g. REPORT_STATS, DRAIN_CLUSTER, HEALTH_CHECK. **Not** game actions (ATTACK, CAST_SPELL, etc.). |
| action_params | bytes or JSON | Payload for the non-game action. |

**Response (target → caller):**

| Field | Type | Description |
|-------|------|-------------|
| request_id | ID | Same as request. |
| result | enum | SUCCESS \| FAILURE. |
| payload | optional | Implementation-defined (e.g. stats JSON). |

Framing (length-prefix or delimiter) to be defined in the wire protocol doc. Message encoding: e.g. msgpack or JSON.

**Note:** Game actions do **not** use this protocol. They use SpacetimeDB reducers; ClusterServer sends RPC_RESULT to the client from the reducer return (CA-01).

---

## 9. Configuration

| Key | Default | Description |
|-----|---------|--------------|
| RPC_PORT_BASE | 9200 | Base port; actual port = 9200 + server index n. |
| RPC_MAX_CONCURRENT | 100 | Max concurrent RPC requests being processed (per server). |
| RPC_REQUEST_TIMEOUT_MS | 5000 | Timeout for caller waiting for response; on timeout caller may send RPC_RESULT FAILURE to client. |

---

## 10. Metrics

| Metric | Type | Labels | Measures |
|--------|------|--------|----------|
| arcane_rpc_requests_total | counter | cluster_id=, action_type=, result=success\|failure | Incoming RPCs and outcome. |
| arcane_rpc_latency_ms | histogram | cluster_id=, action_type= | Time from request received to response sent. |
| arcane_rpc_target_missing_total | counter | cluster_id= | Target entity not owned by this cluster. |
| arcane_rpc_connections | gauge | cluster_id= | Open TCP connections from other ClusterServers. |

---

## 11. Failure Modes

| Failure | Detection | Response |
|---------|-----------|----------|
| Unknown or unsupported action type | Parse or dispatch | Return FAILURE; log. |
| Target cluster unreachable (caller side) | TCP connect or response timeout | Caller retries or gives up; log. |
| Malformed request | Parse error | Close connection or send error response; log. |
| Handler throws or times out | Exception or internal timeout | Return FAILURE; do not crash the server. |

---

## 12. Open Questions

- **Connection reuse:** One TCP connection per (caller, target) pair and multiplex multiple RPCs, or one connection per RPC? Reuse reduces connection churn; single-shot simplifies lifecycle. To be decided in implementation.
- **Ordering:** Should RPCs for the same entity be serialized (one at a time) or allowed to run concurrently? Serializing avoids race conditions; concurrency may be needed for throughput. Likely serialize per entity.
- **Wire doc:** Exact framing and encoding (length-prefix, msgpack vs JSON) to be specified in a dedicated wire protocol doc.

---

*Arcane Engine — IN-05 RPCHandler — Confidential*
