//! IServerPool (IF-02) — cluster server allocation and release.
//!
//! Consumed by manager orchestration (`arcane-infra::ClusterManager`) to provision and reclaim
//! cluster hosts. Implementations live in crates like `arcane-pool`.

use uuid::Uuid;

/// Handle to an allocated cluster server.
#[derive(Clone, Debug)]
pub struct ServerHandle {
    /// Unique identifier for this server instance.
    pub server_id: Uuid,
    /// Hostname or IP address of the server.
    pub host: String,
    /// WebSocket port for client connections.
    pub ws_port: u16,
    /// RPC port for inter-cluster communication.
    pub rpc_port: u16,
    /// Metrics / observability port.
    pub metrics_port: u16,
    /// Monotonic timestamp (seconds since epoch) when the server was allocated.
    pub allocated_at: f64,
}

/// Error from pool operations.
#[derive(Clone, Debug)]
pub struct PoolError {
    /// Machine-readable error code.
    pub code: PoolErrorCode,
    /// Human-readable error detail for logging and debugging.
    pub detail: String,
}

/// Reasons a pool operation can fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolErrorCode {
    /// All servers are allocated; no capacity remaining.
    PoolExhausted,
    /// An allocation request exceeded the timeout.
    AllocationTimeout,
    /// The target server is in an unhealthy state.
    ServerUnhealthy,
}

/// Current pool capacity and health.
#[derive(Clone, Debug)]
pub struct PoolStatus {
    /// Total number of server slots in the pool.
    pub total_capacity: u32,
    /// Number of servers currently free for allocation.
    pub available: u32,
    /// Number of servers currently allocated to clusters.
    pub allocated: u32,
    /// Number of servers in a failed state.
    pub failed: u32,
    /// Minimum desired free capacity; below this triggers scaling.
    pub min_available: u32,
    /// P99 allocation latency in milliseconds.
    pub allocation_p99_ms: f32,
}

/// Contract for allocating and releasing cluster servers. Implemented by LocalPool and ECSPool.
pub trait IServerPool: Send + Sync {
    /// Allocate an available server. Must return within latency contract (e.g. 100ms for LocalPool).
    fn allocate(&self) -> Result<ServerHandle, PoolError>;

    /// Release a server back to the pool. ClusterManager calls after cluster is destroyed.
    fn release(&self, server_id: Uuid) -> Result<(), PoolError>;

    /// Report a failed server and optionally get a replacement.
    fn report_failure(&self, server_id: Uuid, failure_type: FailureType) -> ReplacementHandle;

    /// Current pool status for monitoring.
    fn get_status(&self) -> PoolStatus;
}

/// Category of server failure reported to the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureType {
    /// Server is not reachable over the network.
    Unreachable,
    /// Server process crashed (e.g. segfault, OOM).
    SimulationCrashed,
    /// Server is reachable but operating outside acceptable performance bounds.
    PerformanceDegraded,
}

/// Result of a failure report: an optional replacement server and expected wait time.
#[derive(Clone, Debug)]
pub struct ReplacementHandle {
    /// Replacement server handle, if one was immediately available.
    pub handle: Option<ServerHandle>,
    /// Estimated time (ms) until a replacement is ready, if not immediate.
    pub eta_ms: Option<u32>,
}
