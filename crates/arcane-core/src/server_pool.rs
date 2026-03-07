//! IServerPool (IF-02) — cluster server allocation and release.

use uuid::Uuid;

/// Handle to an allocated cluster server.
#[derive(Clone, Debug)]
pub struct ServerHandle {
    pub server_id: Uuid,
    pub host: String,
    pub ws_port: u16,
    pub rpc_port: u16,
    pub metrics_port: u16,
    pub allocated_at: f64,
}

/// Error from pool operations.
#[derive(Clone, Debug)]
pub struct PoolError {
    pub code: PoolErrorCode,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolErrorCode {
    PoolExhausted,
    AllocationTimeout,
    ServerUnhealthy,
}

/// Current pool capacity and health.
#[derive(Clone, Debug)]
pub struct PoolStatus {
    pub total_capacity: u32,
    pub available: u32,
    pub allocated: u32,
    pub failed: u32,
    pub min_available: u32,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureType {
    Unreachable,
    SimulationCrashed,
    PerformanceDegraded,
}

#[derive(Clone, Debug)]
pub struct ReplacementHandle {
    pub handle: Option<ServerHandle>,
    pub eta_ms: Option<u32>,
}
