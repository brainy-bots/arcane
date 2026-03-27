//! LocalPool — pre-provisioned cluster servers for development (IN-07).

use arcane_core::server_pool::{
    FailureType, IServerPool, PoolError, PoolErrorCode, PoolStatus, ReplacementHandle, ServerHandle,
};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Pre-provisioned pool of cluster server processes (dev). Implements IServerPool.
pub struct LocalPool {
    capacity: u32,
    /// available handles ready to allocate; allocated handles stored by server_id for release
    state: Mutex<PoolState>,
}

struct PoolState {
    available: Vec<ServerHandle>,
    allocated: HashMap<Uuid, ServerHandle>,
}

impl LocalPool {
    pub fn new(capacity: u32) -> Self {
        let mut available = Vec::with_capacity(capacity as usize);
        for i in 0..capacity {
            available.push(ServerHandle {
                server_id: Uuid::new_v4(),
                host: "localhost".to_string(),
                ws_port: 9000 + (i as u16),
                rpc_port: 9100 + (i as u16),
                metrics_port: 9200 + (i as u16),
                allocated_at: 0.0,
            });
        }
        Self {
            capacity,
            state: Mutex::new(PoolState {
                available,
                allocated: HashMap::new(),
            }),
        }
    }
}

impl Default for LocalPool {
    fn default() -> Self {
        Self::new(4)
    }
}

impl IServerPool for LocalPool {
    fn allocate(&self) -> Result<ServerHandle, PoolError> {
        let mut state = self.state.lock().unwrap();
        let handle = match state.available.pop() {
            Some(h) => h,
            None => {
                return Err(PoolError {
                    code: PoolErrorCode::PoolExhausted,
                    detail: "no servers available".to_string(),
                })
            }
        };
        state.allocated.insert(handle.server_id, handle.clone());
        Ok(handle)
    }

    fn release(&self, server_id: Uuid) -> Result<(), PoolError> {
        let mut state = self.state.lock().unwrap();
        let handle = state
            .allocated
            .remove(&server_id)
            .ok_or_else(|| PoolError {
                code: PoolErrorCode::ServerUnhealthy,
                detail: "server_id not allocated".to_string(),
            })?;
        state.available.push(handle);
        Ok(())
    }

    fn report_failure(&self, _server_id: Uuid, _failure_type: FailureType) -> ReplacementHandle {
        ReplacementHandle {
            handle: None,
            eta_ms: None,
        }
    }

    fn get_status(&self) -> PoolStatus {
        let state = self.state.lock().unwrap();
        PoolStatus {
            total_capacity: self.capacity,
            available: state.available.len() as u32,
            allocated: state.allocated.len() as u32,
            failed: 0,
            min_available: 0,
            allocation_p99_ms: 0.0,
        }
    }
}
