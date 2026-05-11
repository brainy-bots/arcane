//! Per-cluster runtime counters exposed to the benchmark harness (and anyone else
//! who wants to verify the cluster is actually processing client traffic).
//!
//! The `ArcaneServerStats` log line has been the only observability signal until
//! now, which lets silent failures (e.g. zero client messages accepted) pass as
//! "successful" runs. This struct pairs with `serve_stats_http` below to expose
//! the same counters over an HTTP `/stats` endpoint on a separate port, so the
//! driver can cross-check "the swarm claims to have sent N messages" against
//! "each cluster actually accepted K of them".

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use uuid::Uuid;

#[derive(Debug, Default)]
pub struct ClusterStats {
    /// WebSocket connections accepted by this cluster's server socket since startup.
    pub ws_accepts: AtomicU64,
    /// PLAYER_STATE messages successfully parsed and forwarded to the cluster.
    pub msgs_player_state: AtomicU64,
    /// GAME_ACTION messages successfully parsed and forwarded to the cluster.
    pub msgs_game_action: AtomicU64,
    /// Incoming text messages that did not parse as either PLAYER_STATE or GAME_ACTION.
    pub parse_failures: AtomicU64,
    /// Inbound text bytes received by this cluster's WebSocket server.
    pub bytes_in: AtomicU64,
    /// Outbound WebSocket bytes sent to connected subscribers (cumulative
    /// across the run). Used alongside `ws_accepts` to compute egress rate.
    pub bytes_out: AtomicU64,
    /// Number of times a subscriber received `RecvError::Lagged(n)` from the
    /// broadcast channel — i.e. the producer emitted frames faster than that
    /// subscriber drained them. Non-zero under fan-out saturation.
    pub broadcast_lagged_events: AtomicU64,
    /// Sum of `n` across all `Lagged(n)` events — total broadcast frames that
    /// subscribers skipped. Scales with both how bad the lag is and how many
    /// subscribers are affected.
    pub broadcast_lagged_frames: AtomicU64,
    /// WebSocket send attempts that returned an error (peer closed or
    /// transport failure). Non-zero under WS-egress saturation or client
    /// drops. Paired with `ws_accepts` to compute active-connection churn.
    pub ws_send_errors: AtomicU64,
    /// Accept loop errors (transient EMFILE/ENFILE, peer resets, kernel
    /// firewall hooks, etc.). Non-zero indicates error recovery happened;
    /// zero indicates the accept loop never hit any condition that required
    /// backoff or continuation.
    pub accept_errors: AtomicU64,
    /// Current entity count observed at the end of the most recent tick.
    pub entities_current: AtomicU64,
    /// Peak entity count observed since startup.
    pub entities_peak: AtomicU64,
    /// Most recent full tick number (see ClusterServer::current_tick).
    pub tick: AtomicU64,
    /// Most recent replication sequence (see ClusterServer::current_seq).
    pub seq: AtomicU64,
    /// Most recent tick elapsed time, in microseconds, for quick health signaling.
    pub last_tick_us: AtomicU64,
    /// Set of entity UUIDs ever observed in a parsed PLAYER_STATE message. The
    /// set is write-only at accept time and only its `len()` is exposed via /stats.
    /// This lets the harness distinguish "many messages share a few ids" from
    /// "many unique ids get dropped after insert" — two very different bugs.
    unique_entity_ids: Mutex<HashSet<Uuid>>,
}

impl ClusterStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record that we observed an entity_id in an accepted PLAYER_STATE message.
    /// Uses a mutex rather than a lock-free set to keep cluster-ws with no new
    /// deps; contention is only at /stats-read time (counting .len()).
    pub fn note_entity_id(&self, id: Uuid) {
        if let Ok(mut set) = self.unique_entity_ids.lock() {
            set.insert(id);
        }
    }

    pub fn unique_entity_ids_count(&self) -> u64 {
        self.unique_entity_ids
            .lock()
            .map(|s| s.len() as u64)
            .unwrap_or(0)
    }

    pub fn set_entities(&self, n: u64) {
        self.entities_current.store(n, Ordering::Relaxed);
        let mut peak = self.entities_peak.load(Ordering::Relaxed);
        while n > peak {
            match self.entities_peak.compare_exchange_weak(
                peak,
                n,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => peak = observed,
            }
        }
    }

    /// Render a flat JSON body for the HTTP stats endpoint. Kept small on purpose
    /// so the harness's SSM/poll path is cheap.
    pub fn to_json(&self, cluster_id: &str) -> String {
        format!(
            r#"{{"cluster_id":"{}","ws_accepts":{},"msgs_player_state":{},"msgs_game_action":{},"parse_failures":{},"bytes_in":{},"bytes_out":{},"broadcast_lagged_events":{},"broadcast_lagged_frames":{},"ws_send_errors":{},"accept_errors":{},"entities_current":{},"entities_peak":{},"unique_entity_ids_seen":{},"tick":{},"seq":{},"last_tick_us":{}}}"#,
            cluster_id,
            self.ws_accepts.load(Ordering::Relaxed),
            self.msgs_player_state.load(Ordering::Relaxed),
            self.msgs_game_action.load(Ordering::Relaxed),
            self.parse_failures.load(Ordering::Relaxed),
            self.bytes_in.load(Ordering::Relaxed),
            self.bytes_out.load(Ordering::Relaxed),
            self.broadcast_lagged_events.load(Ordering::Relaxed),
            self.broadcast_lagged_frames.load(Ordering::Relaxed),
            self.ws_send_errors.load(Ordering::Relaxed),
            self.accept_errors.load(Ordering::Relaxed),
            self.entities_current.load(Ordering::Relaxed),
            self.entities_peak.load(Ordering::Relaxed),
            self.unique_entity_ids_count(),
            self.tick.load(Ordering::Relaxed),
            self.seq.load(Ordering::Relaxed),
            self.last_tick_us.load(Ordering::Relaxed),
        )
    }
}

/// Minimal HTTP server that returns `stats.to_json(cluster_id)` on GET `/stats`
/// and a liveness blob on `/`. Binds on `0.0.0.0:<port>` and never returns; run
/// in its own OS thread with a private Tokio runtime.
///
/// Intentionally hand-rolled (not axum) to avoid adding a dependency to the
/// `cluster-ws` feature. The protocol is one request, one response, no
/// framing — enough for a counters endpoint.
#[cfg(feature = "cluster-ws")]
pub fn serve_stats_http(port: u16, cluster_id: String, stats: Arc<ClusterStats>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("stats tokio runtime");
        rt.block_on(async move {
            let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("cluster stats HTTP bind failed on {}: {}", addr, e);
                    return;
                }
            };
            eprintln!("cluster stats HTTP listening on http://{}/stats", addr);

            loop {
                let (socket, _peer) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let stats = stats.clone();
                let cluster_id = cluster_id.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut socket = socket;
                    let mut buf = [0u8; 1024];
                    let n = match socket.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };
                    let req = &buf[..n];
                    let body = if req.starts_with(b"GET /stats") {
                        stats.to_json(&cluster_id)
                    } else if req.starts_with(b"GET / ") {
                        format!(r#"{{"ok":true,"cluster_id":"{}"}}"#, cluster_id)
                    } else {
                        let _ = socket
                            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                            .await;
                        return;
                    };
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                });
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn set_entities_tracks_peak() {
        let s = ClusterStats::new();
        s.set_entities(10);
        s.set_entities(5);
        s.set_entities(20);
        s.set_entities(15);
        assert_eq!(s.entities_current.load(Ordering::Relaxed), 15);
        assert_eq!(s.entities_peak.load(Ordering::Relaxed), 20);
    }

    #[test]
    fn to_json_includes_all_counters() {
        let s = ClusterStats::new();
        s.ws_accepts.store(3, Ordering::Relaxed);
        s.msgs_player_state.store(100, Ordering::Relaxed);
        s.msgs_game_action.store(7, Ordering::Relaxed);
        s.parse_failures.store(2, Ordering::Relaxed);
        s.bytes_in.store(12345, Ordering::Relaxed);
        s.bytes_out.store(67890, Ordering::Relaxed);
        s.broadcast_lagged_events.store(4, Ordering::Relaxed);
        s.broadcast_lagged_frames.store(17, Ordering::Relaxed);
        s.ws_send_errors.store(1, Ordering::Relaxed);
        s.accept_errors.store(2, Ordering::Relaxed);
        s.set_entities(42);
        s.tick.store(500, Ordering::Relaxed);
        s.seq.store(501, Ordering::Relaxed);
        s.last_tick_us.store(780, Ordering::Relaxed);
        s.note_entity_id(Uuid::from_u128(1));
        s.note_entity_id(Uuid::from_u128(2));
        s.note_entity_id(Uuid::from_u128(1)); // duplicate — still counted as 1
        let json = s.to_json("abc-123");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["cluster_id"], "abc-123");
        assert_eq!(parsed["ws_accepts"], 3);
        assert_eq!(parsed["msgs_player_state"], 100);
        assert_eq!(parsed["msgs_game_action"], 7);
        assert_eq!(parsed["parse_failures"], 2);
        assert_eq!(parsed["bytes_in"], 12345);
        assert_eq!(parsed["bytes_out"], 67890);
        assert_eq!(parsed["broadcast_lagged_events"], 4);
        assert_eq!(parsed["broadcast_lagged_frames"], 17);
        assert_eq!(parsed["ws_send_errors"], 1);
        assert_eq!(parsed["accept_errors"], 2);
        assert_eq!(parsed["entities_current"], 42);
        assert_eq!(parsed["entities_peak"], 42);
        assert_eq!(parsed["unique_entity_ids_seen"], 2);
        assert_eq!(parsed["tick"], 500);
        assert_eq!(parsed["seq"], 501);
        assert_eq!(parsed["last_tick_us"], 780);
    }
}
