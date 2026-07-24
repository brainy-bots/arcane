# IF-02 — IServerPool
**Arcane Node Allocation and Release Interface**

---

| | |
|---|---|
| **Component ID** | IF-02 |
| **Layer** | Infrastructure Interface |
| **Type** | Interface — no implementation, only contract |
| **Purpose** | Define the contract for allocating and releasing Arcane Node processes. Decouples the ArcaneManager from the mechanism by which servers are provisioned so the local pre-provisioned pool (development) and the ECS Fargate pool (production) are interchangeable. |
| **Implementations** | IN-07 ArcaneNodePool (local dev) · ECSServerPool (production, future) |
| **Language** | Rust |
| **Depends On** | None |
| **Required By** | IN-01 ArcaneManager · IN-07 ArcaneNodePool |

---

## 1. Overview

IServerPool abstracts the question of where Arcane Nodes come from. The ArcaneManager needs to allocate a server when a new cluster is created and release it when a cluster is destroyed. It does not need to know whether that server is a pre-spawned local process, a Docker container on the same host, or an ECS Fargate task spinning up in AWS. IServerPool is that abstraction.

The interface is deliberately simple — three methods covering the full lifecycle of a Arcane Node from the ArcaneManager's perspective. The complexity of container orchestration, health checking, and capacity management lives inside the implementation, invisible to the caller.

---

## 2. Responsibilities

- Provide Arcane Node instances on demand within the latency contract
- Accept Arcane Node releases and return them to available capacity
- Report current pool capacity and health
- Handle server failure — mark a server unavailable and provide a replacement
- Maintain enough pre-warmed capacity to meet allocation latency requirements

---

## 3. What It Does NOT Do

- Run the simulation on the Arcane Node — that is ArcaneNode's job
- Make clustering decisions — that is the clustering decision path's job (the global graph partition in `arcane_infra::manager::build_partition_decisions`, ADR-004)
- Route player connections — that is ArcaneManager's job
- Monitor game-level health of a Arcane Node — only infrastructure health (reachable, responsive)
- Persist any game state — Arcane Nodes are stateless infrastructure

---

## 4. Interface Definition

### 4.1 Allocate

```
allocate() -> Result<ServerHandle, PoolError>
```

**Returns:** A `ServerHandle` for an available Arcane Node, or a `PoolError` if none are available within the latency contract.

**Latency contract:** Must return within 100ms for LocalPool. ECSPool may take up to 30 seconds for cold allocation — ArcaneManager must handle async allocation for ECSPool.

```
ServerHandle {
  server_id:   UUID       // stable identifier for this server instance
  host:        string     // hostname or IP
  ws_port:     int        // WebSocket port for player connections (default 8080)
  rpc_port:    int        // TCP port for cross-cluster RPC (default 9200)
  metrics_port: int       // Prometheus metrics endpoint (default 9090)
  allocated_at: float     // timestamp of allocation
}

PoolError {
  code:    POOL_EXHAUSTED | ALLOCATION_TIMEOUT | SERVER_UNHEALTHY
  detail:  string
}
```

---

### 4.2 Release

```
release(server_id: UUID) -> Result<void, PoolError>
```

Marks the server as available for future allocations. The ArcaneManager calls this after a cluster is destroyed and all players have been migrated away. The pool implementation is responsible for resetting any server-side state before making the server available again.

**Post-condition:** Within 5 seconds of release(), the server must be ready to accept a new `allocate()` call. If the server cannot be reset within 5 seconds, it must be removed from the pool and a fresh server used instead.

---

### 4.3 Report Failure

```
report_failure(server_id: UUID, failure_type: FailureType) -> ReplacementHandle
```

Called by ArcaneManager when a Arcane Node becomes unreachable or returns error responses. The pool immediately marks the server as failed, removes it from the available pool permanently, and returns a replacement server handle synchronously if one is available.

```
FailureType {
  UNREACHABLE         // ping timeout or connection refused
  SIMULATION_CRASHED  // process exit or watchdog timeout
  PERFORMANCE_DEGRADED // tick rate below threshold for sustained period
}

ReplacementHandle {
  handle:      ServerHandle?   // null if no replacement immediately available
  eta_ms:      int?            // estimated ms until replacement ready (if null handle)
}
```

---

### 4.4 Pool Status

```
get_status() -> PoolStatus
```

Returns current pool capacity for monitoring and ArcaneManager planning.

```
PoolStatus {
  total_capacity:    int     // total servers the pool can provide
  available:         int     // currently unallocated servers
  allocated:         int     // currently in use
  failed:            int     // servers marked failed since last reset
  min_available:     int     // configured minimum available before warning
  allocation_p99_ms: float   // p99 allocation latency over last 5 minutes
}
```

---

## 5. Internal Structure

### LocalPool (IN-07 ArcaneNodePool)

Pre-spawns N Arcane Node processes at startup. Maintains two lists: `available` and `allocated`. Allocation pops from `available`. Release pushes to `available` after a health ping confirms the server is ready. No dynamic process spawning — if the available list is empty, allocation fails immediately.

```
startup:
  for i in range(POOL_SIZE):
    proc = spawn_arcane_node_process(port=BASE_PORT + i)
    wait_for_ready(proc, timeout=10s)
    available.append(ServerHandle(proc))

allocate():
  if available is empty:
    return PoolError(POOL_EXHAUSTED)
  handle = available.pop()
  allocated[handle.server_id] = handle
  return handle

release(server_id):
  handle = allocated.pop(server_id)
  reset_server(handle)        // sends RESET command via admin HTTP endpoint
  wait_for_ready(handle, timeout=5s)
  available.append(handle)
```

### ECSPool (production, future)

Maintains a warm pool of ECS Fargate tasks. Allocation claims a warm task and registers it with the ArcaneManager. A background process continuously monitors the warm pool size and spawns new tasks when it drops below the minimum threshold. Cold allocation (no warm tasks available) returns a handle with an ETA and triggers async task creation.

---

## 6. Data Ownership

- **Owns:** Registry of all server handles — allocated and available
- **Reads:** Server health status via HTTP ping to each server's metrics endpoint
- **Writes:** Nothing to shared storage — pool state is in-process memory only

---

## 7. Dependencies

None at interface level. LocalPool implementation spawns Rust processes or uses the Docker API. ECSPool depends on the AWS ECS API client via the `aws-sdk-rust` crate.

---

## 8. Configuration

| Key | Default | Description |
|---|---|---|
| `POOL_TYPE` | `local` | Which implementation to use: `local` or `ecs` |
| `POOL_SIZE` | `200` | Number of servers to pre-provision (LocalPool) |
| `POOL_MIN_AVAILABLE` | `20` | Minimum available servers before warning metric fires |
| `BASE_PORT` | `8080` | First WebSocket port — subsequent servers use BASE_PORT + n |
| `RPC_BASE_PORT` | `9200` | First RPC port |
| `SERVER_READY_TIMEOUT_MS` | `10000` | Max wait for a server to become ready after spawn or release |
| `HEALTH_PING_INTERVAL_S` | `10` | How often to health-check available servers |
| `ECS_CLUSTER_ARN` | — | ECS cluster to launch tasks in (ECSPool only) |
| `ECS_TASK_DEFINITION` | — | ECS task definition ARN (ECSPool only) |
| `ECS_WARM_POOL_SIZE` | `50` | Target number of warm ECS tasks (ECSPool only) |

---

## 9. Metrics

| Metric | Type | Labels | Measures |
|---|---|---|---|
| `arcane_pool_available` | gauge | `pool_type=` | Currently available servers |
| `arcane_pool_allocated` | gauge | `pool_type=` | Currently allocated servers |
| `arcane_pool_failed_total` | counter | `failure_type=` | Servers marked failed since startup |
| `arcane_pool_allocate_duration_ms` | histogram | `pool_type=` | Allocation latency |
| `arcane_pool_release_duration_ms` | histogram | | Release and reset latency |
| `arcane_pool_exhausted_total` | counter | | Times allocate() returned POOL_EXHAUSTED |
| `arcane_pool_replacement_eta_ms` | gauge | | ETA for next available server when pool exhausted |

---

## 10. Failure Modes

| Failure | Detection | Response |
|---|---|---|
| Pool exhausted | `available` list empty on allocate() | Return POOL_EXHAUSTED error. ArcaneManager blocks new cluster creation. Log warning. |
| Server fails health check during idle | Periodic ping fails | Remove from available pool. Mark failed. Spawn replacement (LocalPool: skip, ECSPool: trigger). |
| Server crashes during allocation | report_failure() called by ArcaneManager | Mark failed permanently. Return ReplacementHandle with next available. |
| All servers fail simultaneously | Pool exhausted after mass failure | ArcaneManager enters degraded mode — no new clusters, existing clusters continue. Alert fires. |
| Release fails (server unresponsive) | reset_server() timeout | Mark server failed. Do not return to pool. Spawn replacement. Log incident. |

---

## 11. Open Questions

- **Local pool sizing for benchmark:** The demo benchmark needs enough Arcane Nodes for 5000 simulated players at MAX_PLAYERS=20, which is 250 servers minimum. The local machine may not support 250 concurrent processes. Docker resource limits and per-process memory footprint need to be measured before the benchmark pool size is set.
- **ECSPool warm pool cost:** Maintaining 50 warm ECS Fargate tasks continuously is a real cost. The right warm pool size balances allocation latency against idle cost. Needs cost modeling once the per-task resource requirements are known from benchmark results.

---

*Arcane Engine — IF-02 IServerPool — Confidential*
