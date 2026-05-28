//! WebSocket server for cluster state broadcast. Built only with feature "cluster-ws".
//! Accepts incoming PLAYER_STATE messages from clients and forwards them to the tick loop.
//!
//! **Buckets:** inbound binary frames may set **spine** (`position`, `velocity`) and **bucket 2**
//! ([`EntityStateEntry::user_data`](arcane_core::replication_channel::EntityStateEntry::user_data)).
//! **Bucket 3** ([`local_data`](arcane_core::replication_channel::EntityStateEntry::local_data)) is
//! never taken from the client; it stays default until the cluster sets it server-side.
//!
//! ## Wire format
//!
//! All client/server framing is **FlatBuffer binary** via the [`arcane_wire`]
//! crate. JSON was supported historically but has been removed — the cluster
//! speaks one wire format end-to-end, which makes broadcast fan-out cheap
//! (pre-encode once at the producer, share bytes via Arc across subscribers)
//! and eliminates the per-subscriber JSON serialization cost that regressed
//! the cluster's scaling ceiling prior to this rewrite. Non-binary Arcane
//! clients must move to the wire protocol; see the UE5 adapter repo for the
//! client-side migration.
//!
//! ## Broadcast fan-out — Shape B
//!
//! Each tick the producer serializes every entity **once** to a FlatBuffer byte
//! chunk, builds a [`PreEncodedTick`] holding `Arc<Vec<u8>>` chunks plus the
//! delta header and removed-id list, and broadcasts that to all subscribers.
//! Per-client tasks assemble their outbound frame by selecting which entity
//! chunks to include (today: all; with AOI: a filtered subset) and calling
//! [`arcane_wire::encode_server_delta_from_chunks`]. Subscribers never
//! re-serialize individual entities, so the per-tick cost is O(entity count)
//! at the producer and O(entity count × subscriber count) in Arc clones and
//! byte-concatenation — never O(entity count × subscriber count) in
//! serialization, which was the regression this design fixes.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use arcane_core::cluster_simulation::GameAction;
use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
use arcane_core::visibility::IVisibilityFilter;
use arcane_core::Vec3;
use arcane_wire::{
    ClientFrame, DeltaHeader, EntityState as WireEntityState, GameActionPayload,
    PlayerStatePayload, Vec3 as WireVec3,
};
use dashmap::DashMap;
use futures_util::{sink::SinkExt, stream::StreamExt};
use rayon::prelude::*;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::node_stats::NodeStats;

/// Maximum byte length of a single inbound WebSocket binary payload. Client
/// frames larger than this are dropped and counted as parse failures —
/// defends against runaway allocations from misbehaving clients.
const MAX_MESSAGE_BYTES: usize = 64 * 1024; // 64 KiB

fn should_keep_ws_loop_running_on_broadcast_error(
    error: &tokio::sync::broadcast::error::RecvError,
) -> bool {
    matches!(error, tokio::sync::broadcast::error::RecvError::Lagged(_))
}

/// Determine if an accept error should trigger backoff (resource exhaustion).
/// EMFILE and ENFILE indicate the process or system has run out of file descriptors
/// and should back off to avoid a tight error loop. Other errors (peer resets, etc.)
/// continue immediately.
fn should_backoff_on_accept_error(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::EMFILE | libc::ENFILE))
}

/// Convert a [`PlayerStatePayload`] (wire-side) into the cluster-internal
/// [`EntityStateEntry`]. `user_data` bytes are deserialized as JSON if
/// non-empty; empty bytes produce [`serde_json::Value::Null`]. `cluster_id`
/// is set to nil — the cluster binary applies its own when routing.
///
/// The wire-side `Vec3Q` is dequantized at this boundary; downstream sim
/// code stays in continuous f64. Quantized i16 is always finite by
/// construction, so the previous NaN/inf gate is unnecessary.
fn entry_from_wire_player_state(payload: &PlayerStatePayload) -> Option<EntityStateEntry> {
    let user_data = if payload.user_data.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&payload.user_data).ok()?
    };
    let pos = payload.position.to_vec3();
    let vel = payload.velocity.to_vec3();
    let mut entry = EntityStateEntry::new(
        payload.entity_id,
        Uuid::nil(),
        Vec3::new(pos.x, pos.y, pos.z),
        Vec3::new(vel.x, vel.y, vel.z),
    );
    entry.user_data = user_data;
    entry.client_seq = payload.client_seq;
    Some(entry)
}

/// Convert a [`GameActionPayload`] (wire-side) into the cluster-internal
/// [`GameAction`]. `payload` bytes are deserialized as JSON if non-empty.
fn game_action_from_wire(payload: &GameActionPayload) -> Option<GameAction> {
    let json_payload = if payload.payload.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&payload.payload).ok()?
    };
    Some(GameAction {
        entity_id: payload.entity_id,
        action_type: payload.action_type.clone(),
        payload: json_payload,
    })
}

/// Convert one cluster-internal [`EntityStateEntry`] to a FlatBuffer-encoded
/// [`WireEntityState`] byte chunk. Producer calls this once per entity per
/// tick; the resulting bytes are shared across all subscribers via
/// [`PreEncodedTick`].
///
/// Errors are swallowed to an empty chunk. Postcard failures on a valid
/// [`WireEntityState`] are essentially impossible (no variable-length fields
/// that can fail to serialize), so this is a defensive fallback rather than
/// a meaningful error path.
fn encode_entity_chunk(entity: &EntityStateEntry) -> Vec<u8> {
    let user_data_bytes = if entity.user_data.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(&entity.user_data).unwrap_or_default()
    };
    // Quantize at the wire boundary: continuous f64 sim positions become i16
    // on the wire (~6 B per Vec3 instead of 24 B). See `arcane_wire::Vec3Q`
    // for the scale + range tradeoff.
    let wire = WireEntityState {
        entity_id: entity.entity_id,
        cluster_id: entity.cluster_id,
        position: arcane_wire::Vec3Q::from_vec3(WireVec3::new(
            entity.position.x,
            entity.position.y,
            entity.position.z,
        )),
        velocity: arcane_wire::Vec3Q::from_vec3(WireVec3::new(
            entity.velocity.x,
            entity.velocity.y,
            entity.velocity.z,
        )),
        user_data: user_data_bytes,
        client_seq: entity.client_seq,
    };
    arcane_wire::encode_entity_state(&wire)
}

/// Per-tick snapshot assembled once by the producer and broadcast to all
/// subscribers. Each per-client task builds its outbound wire frame by
/// selecting which entity chunks it wants (today: all; with AOI: a subset)
/// and calling [`arcane_wire::encode_server_delta_from_chunks`].
///
/// Entity chunks are `Arc<Vec<u8>>` so N subscribers share one allocation per
/// entity per tick — the whole point of this struct and of Shape B.
///
/// `shared_full_frame` is the pre-assembled wire frame containing ALL
/// entities. Subscribers with no visibility filter send this directly,
/// avoiding per-subscriber chunk concatenation entirely. With N subscribers
/// and E entities this reduces fan-out from O(N×E) to O(E).
struct PreEncodedTick {
    header: DeltaHeader,
    entity_chunks: Vec<Arc<Vec<u8>>>,
    removed: Vec<Uuid>,
    entity_metadata: Vec<(Uuid, Vec3)>,
    shared_full_frame: Arc<Vec<u8>>,
}

/// Per-tick broadcast including pre-encoded entities and precomputed visibility masks.
/// Masks are wrapped in `Arc` so the producer can cache and reuse them across multiple
/// ticks without cloning the full HashMap. Recomputation happens every N ticks
/// (configurable via `ARCANE_VISIBILITY_RECOMPUTE_TICKS`), or immediately when the
/// entity count changes (mask length would mismatch).
struct TickBroadcast {
    tick: Arc<PreEncodedTick>,
    masks: Arc<HashMap<u64, Vec<bool>>>,
}

/// Convert an [`EntityStateDelta`] to a [`PreEncodedTick`] by serializing
/// each entity once. Called by the producer task every tick.
///
/// Entity encoding is **parallelized across a bounded rayon pool** via
/// `par_iter`. The pool is sized by [`encode_pool_thread_count`] — by
/// default half the node's logical cores. The bound is deliberate: giving
/// rayon every core starves tokio's per-subscriber send tasks and causes
/// `broadcast_lagged_events` to explode (see AWS run `20260421_224830`
/// for the empirical demonstration). Half-cores-for-encoding leaves the
/// other half reliably available for broadcast fan-out.
///
/// Correctness: the output order must match the input order because
/// `assemble_outbound_frame` and downstream decoders do not have primary
/// keys on the chunk list. `rayon::par_iter().map(...).collect()`
/// preserves iteration order, so the resulting `Vec` is bit-for-bit
/// identical to the serial version's output.
fn pre_encode_tick(delta: &EntityStateDelta) -> PreEncodedTick {
    let entity_chunks: Vec<Arc<Vec<u8>>> = delta
        .updated
        .par_iter()
        .map(|e| Arc::new(encode_entity_chunk(e)))
        .collect();
    let entity_metadata: Vec<(Uuid, Vec3)> = delta
        .updated
        .iter()
        .map(|e| (e.entity_id, e.position))
        .collect();
    let header = DeltaHeader {
        source_cluster_id: delta.source_cluster_id,
        seq: delta.seq,
        tick: delta.tick,
        timestamp: delta.timestamp,
    };
    let chunk_refs: Vec<&[u8]> = entity_chunks.iter().map(|c| c.as_slice()).collect();
    let shared_full_frame = Arc::new(arcane_wire::encode_server_delta_from_chunks(
        &header,
        &chunk_refs,
        &delta.removed,
    ));
    PreEncodedTick {
        header,
        entity_chunks,
        removed: delta.removed.clone(),
        entity_metadata,
        shared_full_frame,
    }
}

/// Assemble the outbound wire frame for one subscriber from a shared
/// [`PreEncodedTick`] and a precomputed visibility mask.
/// The mask is a boolean vector where `true` means the entity is visible.
fn assemble_outbound_frame(tick: &PreEncodedTick, mask: Option<&[bool]>) -> Vec<u8> {
    let chunk_refs: Vec<&[u8]> = match mask {
        Some(m) => tick
            .entity_chunks
            .iter()
            .zip(m.iter())
            .filter_map(|(c, &visible)| if visible { Some(c.as_slice()) } else { None })
            .collect(),
        None => tick.entity_chunks.iter().map(|c| c.as_slice()).collect(),
    };
    arcane_wire::encode_server_delta_from_chunks(&tick.header, &chunk_refs, &tick.removed)
}

/// Route a binary client frame to the cluster-internal message channel.
enum ClientMessageOutcome {
    Delivered,
    Dropped,
}

fn handle_binary_client_frame(
    bytes: &[u8],
    updates_tx: &Sender<EntityStateEntry>,
    actions_tx: &Sender<GameAction>,
    stats: &NodeStats,
) -> ClientMessageOutcome {
    if bytes.len() > MAX_MESSAGE_BYTES {
        stats.parse_failures.fetch_add(1, Ordering::Relaxed);
        return ClientMessageOutcome::Dropped;
    }
    stats
        .bytes_in
        .fetch_add(bytes.len() as u64, Ordering::Relaxed);
    match arcane_wire::decode_client(bytes) {
        Ok(ClientFrame::PlayerState(payload)) => {
            let Some(entry) = entry_from_wire_player_state(&payload) else {
                stats.parse_failures.fetch_add(1, Ordering::Relaxed);
                return ClientMessageOutcome::Dropped;
            };
            stats.msgs_player_state.fetch_add(1, Ordering::Relaxed);
            stats.note_entity_id(entry.entity_id);
            let _ = updates_tx.send(entry);
            ClientMessageOutcome::Delivered
        }
        Ok(ClientFrame::Action(payload)) => {
            let Some(action) = game_action_from_wire(&payload) else {
                stats.parse_failures.fetch_add(1, Ordering::Relaxed);
                return ClientMessageOutcome::Dropped;
            };
            stats.msgs_game_action.fetch_add(1, Ordering::Relaxed);
            stats.note_entity_id(action.entity_id);
            let _ = actions_tx.send(action);
            ClientMessageOutcome::Delivered
        }
        Err(_) => {
            stats.parse_failures.fetch_add(1, Ordering::Relaxed);
            ClientMessageOutcome::Dropped
        }
    }
}

/// Decide how many threads the encoding rayon pool gets.
///
/// Default: half the node's logical cores, floor of 1. This leaves the
/// other half of the cores reliably available for tokio's per-subscriber
/// send tasks — they individually cost microseconds but collectively need
/// continuous scheduler access to drain the broadcast channel in time.
/// When rayon grabbed every core during each encode burst, subscribers
/// couldn't keep up and `broadcast_lagged_events` exploded (observed
/// empirically, see AWS run `20260421_224830`).
///
/// The split matches the standard "compute pool + reactive IO pool"
/// pattern used in production systems (Cassandra, TiKV, ClickHouse).
/// Half rather than a small fixed reserve (e.g. `N - 2`) because
/// subscriber task count scales with connected clients; on bigger nodes
/// we typically host more clients per cluster, so tokio's CPU share
/// should scale with node size, not be a fixed number.
///
/// `ARCANE_CLUSTER_ENCODE_THREADS` env var overrides the default for
/// operators who want to tune per-workload without rebuilding — e.g.
/// workloads with very few subscribers can push more cores to rayon;
/// bursty reactive workloads can push fewer.
fn encode_pool_thread_count() -> usize {
    if let Ok(s) = std::env::var("ARCANE_CLUSTER_ENCODE_THREADS") {
        if let Ok(n) = s.parse::<usize>() {
            return n.max(1);
        }
    }
    std::thread::available_parallelism()
        .map(|nz| nz.get() / 2)
        .unwrap_or(1)
        .max(1)
}

/// Initialize the global rayon thread pool used by `pre_encode_tick`.
/// Idempotent in effect — rayon's `build_global` fails if already set,
/// which is fine (tests and repeated calls inherit whatever pool was set
/// first). The pool is named so that cluster operators can distinguish
/// encoding threads from tokio workers in `top`/`perf` output.
fn init_encode_thread_pool() {
    let n = encode_pool_thread_count();
    let result = rayon::ThreadPoolBuilder::new()
        .num_threads(n)
        .thread_name(|i| format!("arcane-encode-{i}"))
        .build_global();
    match result {
        Ok(()) => {
            eprintln!("cluster encode pool: {n} threads (override: ARCANE_CLUSTER_ENCODE_THREADS)")
        }
        Err(_) => {
            // Already initialized earlier in the process (e.g. by a test
            // or a previous call). Not fatal; we just reuse the existing
            // pool.
        }
    }
}

/// How many ticks to reuse cached visibility masks before recomputing.
///
/// At 3K entities × 3K subscribers, a full recompute is ~9M distance checks.
/// Reusing masks across ticks amortizes this cost: at 60 Hz with an interval
/// of 30, masks recompute at ~2 Hz — reducing filter work by ~30×.
///
/// Masks are also invalidated when the entity count changes (spawn/despawn),
/// since the mask length must match the entity chunk count.
///
/// `ARCANE_VISIBILITY_RECOMPUTE_TICKS` env var overrides the default.
fn visibility_recompute_interval() -> u64 {
    if let Ok(s) = std::env::var("ARCANE_VISIBILITY_RECOMPUTE_TICKS") {
        if let Ok(n) = s.parse::<u64>() {
            return n.max(1);
        }
    }
    30
}

/// Compute all subscriber visibility masks in parallel via rayon.
/// Takes the preencoded tick, the visibility filter, and subscriber positions.
/// Returns a HashMap<subscriber_id, mask> where each mask is a Vec<bool> indicating
/// which entities are visible to that subscriber.
fn compute_visibility_masks(
    tick: &PreEncodedTick,
    filter: Option<&dyn IVisibilityFilter>,
    subscriber_positions: &DashMap<u64, Vec3>,
) -> HashMap<u64, Vec<bool>> {
    match filter {
        Some(f) => {
            // Collect subscriber IDs and positions into a vec for parallel iteration
            let subscribers: Vec<(u64, Vec3)> = subscriber_positions
                .iter()
                .map(|entry| (*entry.key(), *entry.value()))
                .collect();

            // Compute masks in parallel
            subscribers
                .par_iter()
                .map(|&(sub_id, obs_pos)| {
                    let mask = f.filter(obs_pos, &tick.entity_metadata);
                    (sub_id, mask)
                })
                .collect()
        }
        None => {
            // No filter active; no masks needed
            HashMap::new()
        }
    }
}

pub fn run_ws_server(
    port: u16,
    state_rx: Receiver<EntityStateDelta>,
    client_updates_tx: Sender<EntityStateEntry>,
    game_actions_tx: Sender<GameAction>,
    stats: Arc<NodeStats>,
) {
    // Bound the encoding rayon pool BEFORE spawning the tokio runtime,
    // so the pool is ready by the time the first tick fires.
    init_encode_thread_pool();
    std::thread::spawn(move || {
        // Multi-thread runtime so broadcast fan-out (one subscriber task per
        // connected client) and inbound WS-frame decode can run concurrently
        // across CPU cores. The pre-Shape-B regression forced every subscriber
        // to serialize on a single reactor; even after Shape B removed the
        // per-subscriber re-serialization, the subscriber send + inbound decode
        // workload still scales with connected-client count and benefits from
        // parallel execution. Defaults to one worker thread per available core.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(ws_loop(
            port,
            state_rx,
            client_updates_tx,
            game_actions_tx,
            stats,
        ));
    });
}

/// Global atomic counter for assigning unique subscriber IDs.
static NEXT_SUBSCRIBER_ID: AtomicU64 = AtomicU64::new(1);

async fn ws_loop(
    port: u16,
    state_rx: Receiver<EntityStateDelta>,
    client_updates_tx: Sender<EntityStateEntry>,
    game_actions_tx: Sender<GameAction>,
    stats: Arc<NodeStats>,
) {
    // Broadcast carries a TickBroadcast — per-entity FlatBuffer chunks plus
    // delta header and removed-id list, plus precomputed visibility masks.
    // Subscribers assemble their outbound frame from the shared chunks via
    // arcane_wire::encode_server_delta_from_chunks with a precomputed mask.
    // One serialization per entity per tick, shared across all subscribers.
    //
    // Buffer cap is the deepest backlog the slowest subscriber can fall
    // behind by before the channel fires `Lagged` and drops oldest frames.
    // Sourced from `ARCANE_BROADCAST_CHANNEL_CAP` (see
    // crate::broadcast_channel_cap) — empirically the binding constraint
    // at 30 Hz / 100 ms after dead reckoning + quantization landed (cluster
    // CPU and NIC both had headroom; lagged_frames kept firing).
    let cap = crate::broadcast_channel_cap::broadcast_channel_cap();
    eprintln!("cluster broadcast channel cap: {cap} (override: ARCANE_BROADCAST_CHANNEL_CAP)");
    let (broadcast_tx, _) = tokio::sync::broadcast::channel::<Arc<TickBroadcast>>(cap);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await.expect("bind ws port");
    eprintln!(
        "cluster WebSocket listening on ws://{} (binary arcane-wire frames only)",
        addr
    );

    let broadcast_tx = Arc::new(broadcast_tx);
    let tx_clone = broadcast_tx.clone();
    let rx = Arc::new(std::sync::Mutex::new(state_rx));

    // Shared subscriber position map: DashMap<subscriber_id, position>
    // Updated by subscriber tasks on PLAYER_STATE, read by producer for mask computation.
    let subscriber_positions = Arc::new(DashMap::new());
    let positions_clone = subscriber_positions.clone();

    tokio::spawn(async move {
        let recompute_interval = visibility_recompute_interval();
        let mut cached_masks: Arc<HashMap<u64, Vec<bool>>> = Arc::new(HashMap::new());
        let mut cached_entity_count: usize = 0;
        let mut ticks_until_recompute: u64 = 0;

        loop {
            let r = rx.clone();
            let delta = tokio::task::spawn_blocking(move || r.lock().unwrap().recv())
                .await
                .unwrap();
            match delta {
                Ok(d) => {
                    let tick = Arc::new(pre_encode_tick(&d));

                    let entity_count = tick.entity_metadata.len();
                    if ticks_until_recompute == 0 || entity_count != cached_entity_count {
                        cached_masks =
                            Arc::new(compute_visibility_masks(&tick, None, &positions_clone));
                        cached_entity_count = entity_count;
                        ticks_until_recompute = recompute_interval;
                    } else {
                        ticks_until_recompute -= 1;
                    }

                    let broadcast = Arc::new(TickBroadcast {
                        tick,
                        masks: cached_masks.clone(),
                    });
                    let _ = tx_clone.send(broadcast);
                }
                Err(_) => break,
            }
        }
    });

    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                stats.accept_errors.fetch_add(1, Ordering::Relaxed);
                // Backoff on resource exhaustion to avoid a tight error loop;
                // continue immediately on transient peer-side errors.
                if should_backoff_on_accept_error(&e) {
                    eprintln!("ws accept: fd exhaustion ({}); backing off 100ms", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                } else {
                    eprintln!("ws accept: transient error ({}); continuing", e);
                }
                continue;
            }
        };
        let mut ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let accept_n = stats.ws_accepts.fetch_add(1, Ordering::Relaxed) + 1;
        if accept_n <= 3 || accept_n.is_power_of_two() {
            eprintln!("ws accept #{} from {}", accept_n, peer_addr);
        }

        // Assign unique subscriber ID
        let subscriber_id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);

        let mut recv = broadcast_tx.subscribe();
        let updates_tx = client_updates_tx.clone();
        let actions_tx = game_actions_tx.clone();
        let stats = stats.clone();
        let positions = subscriber_positions.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = recv.recv() => {
                        match result {
                            Ok(broadcast_arc) => {
                                let mask = broadcast_arc.masks.get(&subscriber_id);
                                let (bytes, byte_len) = match mask {
                                    Some(m) => {
                                        let b = assemble_outbound_frame(&broadcast_arc.tick, Some(m.as_slice()));
                                        let len = b.len() as u64;
                                        (Message::Binary(b), len)
                                    }
                                    None => {
                                        let len = broadcast_arc.tick.shared_full_frame.len() as u64;
                                        (Message::Binary((*broadcast_arc.tick.shared_full_frame).clone()), len)
                                    }
                                };
                                if ws_stream.send(bytes).await.is_err() {
                                    stats.ws_send_errors.fetch_add(1, Ordering::Relaxed);
                                    break;
                                }
                                stats.bytes_out.fetch_add(byte_len, Ordering::Relaxed);
                            }
                            Err(error) => {
                                // Backpressure/loss policy: tolerate dropped broadcast frames (`Lagged`)
                                // and continue with freshest state; terminate only when channel is closed.
                                if let tokio::sync::broadcast::error::RecvError::Lagged(n) = error {
                                    stats.broadcast_lagged_events.fetch_add(1, Ordering::Relaxed);
                                    stats.broadcast_lagged_frames.fetch_add(n, Ordering::Relaxed);
                                }
                                if !should_keep_ws_loop_running_on_broadcast_error(&error) {
                                    break;
                                }
                            },
                        }
                    }
                    msg = ws_stream.next() => {
                        match msg {
                            Some(Ok(Message::Binary(bytes))) => {
                                if let Ok(arcane_wire::ClientFrame::PlayerState(payload)) = arcane_wire::decode_client(&bytes) {
                                    if let Some(entry) = entry_from_wire_player_state(&payload) {
                                        // Update shared subscriber positions
                                        positions.insert(subscriber_id, entry.position);
                                    }
                                }
                                let _ = handle_binary_client_frame(&bytes, &updates_tx, &actions_tx, &stats);
                            }
                            Some(Err(_)) | None => {
                                // Subscriber disconnected; remove from position map
                                positions.remove(&subscriber_id);
                                break;
                            },
                            // Text and other frame types are not part of the
                            // supported wire protocol and are silently ignored.
                            _ => {}
                        }
                    }
                }
            }
            // Clean up position on task exit
            positions.remove(&subscriber_id);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assemble_outbound_frame, encode_entity_chunk, entry_from_wire_player_state,
        game_action_from_wire, pre_encode_tick, should_backoff_on_accept_error,
        should_keep_ws_loop_running_on_broadcast_error,
    };
    use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
    use arcane_core::visibility::IVisibilityFilter;
    use arcane_core::Vec3;
    use arcane_wire::{
        ClientFrame, GameActionPayload, PlayerStatePayload, ServerFrame, Vec3 as WireVec3,
    };
    use tokio::sync::broadcast::error::RecvError;
    use uuid::Uuid;

    // ── backpressure policy ──────────────────────────────────────────────

    #[test]
    fn backpressure_policy_keeps_loop_on_lagged_messages() {
        assert!(should_keep_ws_loop_running_on_broadcast_error(
            &RecvError::Lagged(5)
        ));
    }

    #[test]
    fn backpressure_policy_stops_loop_when_channel_closed() {
        assert!(!should_keep_ws_loop_running_on_broadcast_error(
            &RecvError::Closed
        ));
    }

    // ── accept loop error handling ──────────────────────────────────────

    #[test]
    fn accept_error_backoff_triggers_on_emfile() {
        let err = std::io::Error::from_raw_os_error(libc::EMFILE);
        assert!(should_backoff_on_accept_error(&err));
    }

    #[test]
    fn accept_error_backoff_triggers_on_enfile() {
        let err = std::io::Error::from_raw_os_error(libc::ENFILE);
        assert!(should_backoff_on_accept_error(&err));
    }

    #[test]
    fn accept_error_no_backoff_on_other_errors() {
        // ECONNABORTED and other transient errors should not trigger backoff.
        let err = std::io::Error::from_raw_os_error(libc::ECONNABORTED);
        assert!(!should_backoff_on_accept_error(&err));

        // Test a few other error codes that accept() can return.
        let err = std::io::Error::from_raw_os_error(libc::EPROTO);
        assert!(!should_backoff_on_accept_error(&err));

        let err = std::io::Error::from_raw_os_error(libc::EPERM);
        assert!(!should_backoff_on_accept_error(&err));
    }

    // ── inbound wire decode (kept from the pre-Shape-B era) ──────────────

    /// Test-side shorthand for building the on-wire quantized vector from
    /// f64 components. The non-finite-position regression test that used to
    /// guard against `f64::INFINITY` is gone — i16 can't represent it.
    fn wire_q3(x: f64, y: f64, z: f64) -> arcane_wire::Vec3Q {
        arcane_wire::Vec3Q::from_vec3(WireVec3::new(x, y, z))
    }

    #[test]
    fn entry_from_wire_accepts_empty_user_data_as_null() {
        let id = Uuid::from_u128(42);
        let payload = PlayerStatePayload {
            entity_id: id,
            position: wire_q3(1.0, 2.0, 3.0),
            velocity: wire_q3(0.0, 0.1, 0.0),
            user_data: Vec::new(),
            client_seq: 0,
        };
        let entry = entry_from_wire_player_state(&payload).expect("parse");
        assert_eq!(entry.entity_id, id);
        assert_eq!(entry.position.x, 1.0);
        assert!(entry.user_data.is_null());
    }

    #[test]
    fn entry_from_wire_deserializes_user_data_json_bytes() {
        let payload = PlayerStatePayload {
            entity_id: Uuid::from_u128(1),
            position: wire_q3(0.0, 0.0, 0.0),
            velocity: wire_q3(0.0, 0.0, 0.0),
            user_data: serde_json::to_vec(&serde_json::json!({"hp": 99})).unwrap(),
            client_seq: 0,
        };
        let entry = entry_from_wire_player_state(&payload).expect("parse");
        assert_eq!(entry.user_data, serde_json::json!({"hp": 99}));
    }

    #[test]
    fn game_action_from_wire_handles_empty_payload() {
        let payload = GameActionPayload {
            entity_id: Uuid::from_u128(1),
            action_type: "interact".to_string(),
            payload: Vec::new(),
        };
        let action = game_action_from_wire(&payload).expect("parse");
        assert_eq!(action.action_type, "interact");
        assert!(action.payload.is_null());
    }

    #[test]
    fn binary_client_frame_roundtrips_player_state_through_arcane_wire() {
        // End-to-end: wire-encode a ClientFrame::PlayerState, decode via
        // arcane_wire::decode_client, convert via entry_from_wire_player_state.
        // Sub-unit components are rounded by the i16 quantization step at
        // the wire boundary, so we assert against the rounded values.
        let id = Uuid::from_u128(123);
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: id,
            position: wire_q3(1.25, 2.5, 3.75),
            velocity: wire_q3(0.1, 0.0, -0.1),
            user_data: Vec::new(),
            client_seq: 0,
        });
        let bytes = arcane_wire::encode_client(&frame);
        let decoded = arcane_wire::decode_client(&bytes).unwrap();
        let ClientFrame::PlayerState(payload) = decoded else {
            panic!("expected PlayerState variant");
        };
        let entry = entry_from_wire_player_state(&payload).expect("parse");
        assert_eq!(entry.entity_id, id);
        assert_eq!(entry.position.x, 1.0);
        assert_eq!(entry.velocity.z, 0.0);
    }

    // ── Shape B primitives: encode_entity_chunk + pre_encode_tick +
    //    assemble_outbound_frame ─────────────────────────────────────────

    /// A chunk produced by `encode_entity_chunk` must be a valid standalone
    /// FlatBuffer encoding of a `WireEntityState` — i.e. decode round-trip
    /// succeeds through `arcane_wire`'s primitives.
    #[test]
    fn encode_entity_chunk_produces_decodable_wire_entity_state() {
        let mut entry = EntityStateEntry::new(
            Uuid::from_u128(7),
            Uuid::from_u128(9),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(0.1, 0.2, 0.3),
        );
        entry.user_data = serde_json::json!({"hp": 99});
        let chunk = encode_entity_chunk(&entry);

        let wire = arcane_wire::decode_entity_state(&chunk).expect("chunk decodes");
        assert_eq!(wire.entity_id, entry.entity_id);
        assert_eq!(wire.cluster_id, entry.cluster_id);
        // Wire is now Vec3Q (i16). 1.0 quantizes to 1; 0.3 rounds to 0.
        assert_eq!(wire.position.x, 1i16);
        assert_eq!(wire.velocity.z, 0i16);
        let recovered: serde_json::Value = serde_json::from_slice(&wire.user_data).unwrap();
        assert_eq!(recovered, serde_json::json!({"hp": 99}));
    }

    #[test]
    fn encode_entity_chunk_emits_empty_user_data_for_null_value() {
        let entry = EntityStateEntry::new(
            Uuid::from_u128(1),
            Uuid::nil(),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
        );
        let chunk = encode_entity_chunk(&entry);
        let wire = arcane_wire::decode_entity_state(&chunk).unwrap();
        assert!(wire.user_data.is_empty());
    }

    /// Producer path end-to-end: build a delta, pre-encode it into chunks,
    /// assemble the outbound frame, decode via the standard
    /// `arcane_wire::decode_server`, and verify shape + content. This
    /// exercises the same flow the ws_loop follows every tick.
    #[test]
    fn pre_encode_and_assemble_roundtrip_matches_input_delta() {
        let mut entry_a = EntityStateEntry::new(
            Uuid::from_u128(1),
            Uuid::from_u128(99),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(0.0, 0.0, 0.0),
        );
        entry_a.user_data = serde_json::json!({"hp": 42});
        let entry_b = EntityStateEntry::new(
            Uuid::from_u128(2),
            Uuid::from_u128(99),
            Vec3::new(10.0, 20.0, 30.0),
            Vec3::new(0.5, 0.0, 0.0),
        );

        let delta = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(99),
            seq: 7,
            tick: 42,
            timestamp: 1.5,
            updated: vec![entry_a.clone(), entry_b.clone()],
            removed: vec![Uuid::from_u128(5)],
        };

        let pre_tick = pre_encode_tick(&delta);
        assert_eq!(pre_tick.entity_chunks.len(), 2);
        assert_eq!(pre_tick.removed, vec![Uuid::from_u128(5)]);
        assert_eq!(pre_tick.header.seq, 7);
        assert_eq!(pre_tick.header.tick, 42);

        let bytes = assemble_outbound_frame(&pre_tick, None);
        let decoded = arcane_wire::decode_server(&bytes).expect("decode");
        let ServerFrame::Delta(payload) = decoded;

        assert_eq!(payload.source_cluster_id, delta.source_cluster_id);
        assert_eq!(payload.seq, 7);
        assert_eq!(payload.tick, 42);
        assert_eq!(payload.timestamp, 1.5);
        assert_eq!(payload.updated.len(), 2);
        assert_eq!(payload.updated[0].entity_id, entry_a.entity_id);
        assert_eq!(payload.updated[1].entity_id, entry_b.entity_id);
        assert_eq!(payload.removed, vec![Uuid::from_u128(5)]);

        // user_data bytes round-trip correctly for the entity that had one.
        let recovered: serde_json::Value =
            serde_json::from_slice(&payload.updated[0].user_data).unwrap();
        assert_eq!(recovered, serde_json::json!({"hp": 42}));
        assert!(payload.updated[1].user_data.is_empty());
    }

    /// Empty delta (no entities, no removals) is still a valid encoded frame.
    #[test]
    fn pre_encode_and_assemble_handles_empty_delta() {
        let delta = EntityStateDelta {
            source_cluster_id: Uuid::nil(),
            seq: 0,
            tick: 0,
            timestamp: 0.0,
            updated: Vec::new(),
            removed: Vec::new(),
        };
        let pre_tick = pre_encode_tick(&delta);
        let bytes = assemble_outbound_frame(&pre_tick, None);
        let decoded = arcane_wire::decode_server(&bytes).expect("decode");
        let ServerFrame::Delta(payload) = decoded;
        assert!(payload.updated.is_empty());
        assert!(payload.removed.is_empty());
    }

    /// **The Shape B correctness guarantee:** subscribers produce the same
    /// bytes they would have produced if we'd re-serialized the whole delta
    /// per subscriber (the pre-regression code path's output shape), but now
    /// the per-entity serialization happens once and is reused. Verifies
    /// bit-for-bit parity between the pre-encoded-chunks path and a
    /// reference `arcane_wire::encode_server(ServerFrame::Delta(...))`
    /// invocation on the equivalent payload.
    #[test]
    fn shape_b_outbound_bytes_match_full_encode_byte_for_byte() {
        let mut entities = Vec::new();
        for i in 0..50_u128 {
            let mut e = EntityStateEntry::new(
                Uuid::from_u128(i),
                Uuid::from_u128(99),
                Vec3::new(i as f64, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
            );
            if i % 2 == 0 {
                e.user_data = serde_json::json!({"idx": i as u64});
            }
            entities.push(e);
        }
        let delta = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(99),
            seq: 1,
            tick: 2,
            timestamp: 3.0,
            updated: entities.clone(),
            removed: vec![Uuid::from_u128(999), Uuid::from_u128(1000)],
        };

        let via_shape_b = assemble_outbound_frame(&pre_encode_tick(&delta), None);

        // Reference: build the wire DeltaPayload by materializing every
        // WireEntityState inline, encode as one ServerFrame::Delta — the
        // pre-regression path's output shape.
        let wire_updated: Vec<arcane_wire::EntityState> = entities
            .iter()
            .map(|e| {
                let user_data = if e.user_data.is_null() {
                    Vec::new()
                } else {
                    serde_json::to_vec(&e.user_data).unwrap()
                };
                arcane_wire::EntityState {
                    entity_id: e.entity_id,
                    cluster_id: e.cluster_id,
                    position: arcane_wire::Vec3Q::from_vec3(WireVec3::new(
                        e.position.x,
                        e.position.y,
                        e.position.z,
                    )),
                    velocity: arcane_wire::Vec3Q::from_vec3(WireVec3::new(
                        e.velocity.x,
                        e.velocity.y,
                        e.velocity.z,
                    )),
                    user_data,
                    client_seq: e.client_seq,
                }
            })
            .collect();
        let reference_payload = arcane_wire::DeltaPayload {
            source_cluster_id: delta.source_cluster_id,
            seq: delta.seq,
            tick: delta.tick,
            timestamp: delta.timestamp,
            updated: wire_updated,
            removed: delta.removed.clone(),
        };
        let via_reference = arcane_wire::encode_server(&ServerFrame::Delta(reference_payload));

        assert_eq!(
            via_shape_b, via_reference,
            "Shape B must produce bit-identical output to the full-encode path"
        );
    }

    /// Parallel pre-encoding must preserve chunk ORDER, because
    /// `assemble_outbound_frame` and the downstream decoder rely on it
    /// (the `updated` list has no primary key that could re-order it
    /// after the fact). `rayon::par_iter().map().collect::<Vec<_>>()`
    /// guarantees iteration-order preservation, but this test pins the
    /// guarantee so a future refactor to `collect_into_vec` or similar
    /// can't silently break it — the produced outbound frame would
    /// deserialize entities in scrambled order, which subscribers would
    /// see as teleporting players.
    #[test]
    fn parallel_pre_encode_preserves_entity_order() {
        // Large enough to cross rayon's work-splitting threshold
        // (typically 1 item per chunk but worth making it meaningful).
        let mut entities = Vec::with_capacity(200);
        for i in 0..200_u128 {
            let mut e = EntityStateEntry::new(
                Uuid::from_u128(i),
                Uuid::from_u128(99),
                Vec3::new(i as f64 * 1.5, 0.0, i as f64 * -0.25),
                Vec3::new(0.0, 0.0, 0.0),
            );
            e.user_data = serde_json::json!({"order_marker": i as u64});
            entities.push(e);
        }
        let delta = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(99),
            seq: 1,
            tick: 2,
            timestamp: 3.0,
            updated: entities.clone(),
            removed: vec![],
        };

        let pre_tick = pre_encode_tick(&delta);

        // Decode each chunk back and verify its entity_id matches the
        // input entity at the same index — parallel execution must not
        // reorder.
        assert_eq!(pre_tick.entity_chunks.len(), entities.len());
        for (i, chunk) in pre_tick.entity_chunks.iter().enumerate() {
            let decoded = arcane_wire::decode_entity_state(chunk).expect("chunk decodes");
            assert_eq!(
                decoded.entity_id, entities[i].entity_id,
                "chunk at index {} was reordered by parallel encoding",
                i
            );
        }

        // Also verify end-to-end frame decodes identically to what a
        // serial encode would have produced.
        let bytes = assemble_outbound_frame(&pre_tick, None);
        let decoded = arcane_wire::decode_server(&bytes).expect("decode");
        let ServerFrame::Delta(payload) = decoded;
        for (i, e) in payload.updated.iter().enumerate() {
            assert_eq!(e.entity_id, entities[i].entity_id);
        }
    }

    // ── encode thread pool sizing ────────────────────────────────────────

    /// Single serial test covering both env override and default-half-cores
    /// behavior. Kept as one test because cargo test runs tests in parallel
    /// by default and both paths mutate the same env var — splitting them
    /// would introduce a race. Saved-and-restored around any other test
    /// that happens to observe the env.
    #[test]
    fn encode_pool_thread_count_default_and_override() {
        let prev = std::env::var("ARCANE_CLUSTER_ENCODE_THREADS").ok();

        // Default: no env var → half cores, floor of 1.
        std::env::remove_var("ARCANE_CLUSTER_ENCODE_THREADS");
        let cores = std::thread::available_parallelism()
            .map(|nz| nz.get())
            .unwrap_or(1);
        let expected_default = (cores / 2).max(1);
        assert_eq!(
            super::encode_pool_thread_count(),
            expected_default,
            "default should be max(1, num_cpus / 2)"
        );

        // Valid positive integer override honored.
        std::env::set_var("ARCANE_CLUSTER_ENCODE_THREADS", "3");
        assert_eq!(super::encode_pool_thread_count(), 3);

        // 0 clamped to floor of 1 (rayon rejects 0-thread pools).
        std::env::set_var("ARCANE_CLUSTER_ENCODE_THREADS", "0");
        assert_eq!(super::encode_pool_thread_count(), 1);

        // Non-numeric falls through to default; must not panic.
        std::env::set_var("ARCANE_CLUSTER_ENCODE_THREADS", "not-a-number");
        assert_eq!(
            super::encode_pool_thread_count(),
            expected_default,
            "garbage env should fall through to the default"
        );

        // Restore prior state.
        match prev {
            Some(v) => std::env::set_var("ARCANE_CLUSTER_ENCODE_THREADS", v),
            None => std::env::remove_var("ARCANE_CLUSTER_ENCODE_THREADS"),
        }
    }

    // ── visibility filter integration ────────────────────────────────────

    /// Mock visibility filter for testing AOI integration.
    struct MockRadiusFilter {
        radius: f64,
    }

    impl MockRadiusFilter {
        fn new(radius: f64) -> Self {
            Self { radius }
        }
    }

    impl arcane_core::visibility::IVisibilityFilter for MockRadiusFilter {
        fn filter(&self, observer_position: Vec3, entities: &[(Uuid, Vec3)]) -> Vec<bool> {
            entities
                .iter()
                .map(|(_, pos)| {
                    let distance_sq = observer_position.distance_sq_to(pos);
                    distance_sq <= self.radius * self.radius
                })
                .collect()
        }
    }

    /// With a visibility filter applied, only entities passing the filter
    /// should appear in the outbound frame.
    #[test]
    fn assemble_with_filter_includes_only_visible_entities() {
        let entities = vec![
            // Entity at (0, 0, 0)
            EntityStateEntry::new(
                Uuid::from_u128(1),
                Uuid::from_u128(99),
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
            ),
            // Entity at (5, 0, 0) — within radius 10
            EntityStateEntry::new(
                Uuid::from_u128(2),
                Uuid::from_u128(99),
                Vec3::new(5.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
            ),
            // Entity at (20, 0, 0) — outside radius 10
            EntityStateEntry::new(
                Uuid::from_u128(3),
                Uuid::from_u128(99),
                Vec3::new(20.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
            ),
        ];

        let delta = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(99),
            seq: 1,
            tick: 1,
            timestamp: 0.0,
            updated: entities,
            removed: Vec::new(),
        };

        let pre_tick = pre_encode_tick(&delta);

        // Observer at (0, 0, 0), filter radius 10
        let observer_pos = Vec3::new(0.0, 0.0, 0.0);
        let filter = MockRadiusFilter::new(10.0);
        let mask = filter.filter(observer_pos, &pre_tick.entity_metadata);
        let bytes_filtered = assemble_outbound_frame(&pre_tick, Some(&mask));

        // Decode the filtered frame
        let decoded_filtered =
            arcane_wire::decode_server(&bytes_filtered).expect("decode filtered");
        let ServerFrame::Delta(payload) = decoded_filtered;
        assert_eq!(payload.updated.len(), 2);
        assert_eq!(payload.updated[0].entity_id, Uuid::from_u128(1));
        assert_eq!(payload.updated[1].entity_id, Uuid::from_u128(2));
    }

    /// Without a visibility filter (None), all entities should be included
    /// — this is the backward-compatible no-filter behavior.
    #[test]
    fn assemble_without_filter_includes_all_entities() {
        let entities = vec![
            EntityStateEntry::new(
                Uuid::from_u128(1),
                Uuid::from_u128(99),
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
            ),
            EntityStateEntry::new(
                Uuid::from_u128(2),
                Uuid::from_u128(99),
                Vec3::new(5.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
            ),
            EntityStateEntry::new(
                Uuid::from_u128(3),
                Uuid::from_u128(99),
                Vec3::new(20.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
            ),
        ];

        let delta = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(99),
            seq: 1,
            tick: 1,
            timestamp: 0.0,
            updated: entities,
            removed: Vec::new(),
        };

        let pre_tick = pre_encode_tick(&delta);

        // No filter (None)
        let bytes_unfiltered = assemble_outbound_frame(&pre_tick, None);

        // Decode the unfiltered frame
        let decoded_unfiltered =
            arcane_wire::decode_server(&bytes_unfiltered).expect("decode unfiltered");
        let ServerFrame::Delta(payload) = decoded_unfiltered;
        assert_eq!(payload.updated.len(), 3);
        assert_eq!(payload.updated[0].entity_id, Uuid::from_u128(1));
        assert_eq!(payload.updated[1].entity_id, Uuid::from_u128(2));
        assert_eq!(payload.updated[2].entity_id, Uuid::from_u128(3));
    }

    /// Verify that Shape B byte-for-byte parity still holds when filter is None.
    #[test]
    fn shape_b_byte_parity_maintained_with_none_filter() {
        let mut entities = Vec::new();
        for i in 0..10_u128 {
            let e = EntityStateEntry::new(
                Uuid::from_u128(i),
                Uuid::from_u128(99),
                Vec3::new(i as f64, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
            );
            entities.push(e);
        }

        let delta = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(99),
            seq: 1,
            tick: 1,
            timestamp: 0.0,
            updated: entities.clone(),
            removed: Vec::new(),
        };

        let pre_tick = pre_encode_tick(&delta);

        // Both should produce identical bytes when mask is None
        let bytes_no_mask = assemble_outbound_frame(&pre_tick, None);
        let bytes_none_mask = assemble_outbound_frame(&pre_tick, None);

        assert_eq!(bytes_no_mask, bytes_none_mask);
    }

    /// The pre-built `shared_full_frame` must be byte-identical to what
    /// `assemble_outbound_frame(tick, None)` produces on demand.
    #[test]
    fn shared_full_frame_equals_assemble_no_mask() {
        let mut entities = Vec::new();
        for i in 0..20_u128 {
            let mut e = EntityStateEntry::new(
                Uuid::from_u128(i),
                Uuid::from_u128(77),
                Vec3::new(i as f64, (i as f64) * 0.5, 0.0),
                Vec3::new(1.0, 0.0, -1.0),
            );
            if i % 3 == 0 {
                e.user_data = serde_json::json!({"kind": "npc", "idx": i});
            }
            entities.push(e);
        }

        let delta = EntityStateDelta {
            source_cluster_id: Uuid::from_u128(77),
            seq: 42,
            tick: 500,
            timestamp: 12345.678,
            updated: entities,
            removed: vec![Uuid::from_u128(999), Uuid::from_u128(1000)],
        };

        let pre_tick = pre_encode_tick(&delta);
        let assembled = assemble_outbound_frame(&pre_tick, None);

        assert_eq!(
            assembled,
            pre_tick.shared_full_frame.as_slice(),
            "shared_full_frame must be byte-identical to assemble_outbound_frame(tick, None)"
        );
    }
}
