//! RPCHandler (IN-05) — optional TCP endpoint for non-game server-to-server RPC. Scaffolding.

/// Optional. TCP server for non-game RPC (admin, tooling).
pub struct RpcHandler {
    port: u16,
    running: std::sync::atomic::AtomicBool,
}

impl RpcHandler {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Start listening. Returns when server is bound.
    pub fn start(&self) -> Result<(), String> {
        self.running
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Stop listening and close connections.
    pub fn stop(&self) {
        self.running
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether the handler is currently running (for tests).
    pub fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}
