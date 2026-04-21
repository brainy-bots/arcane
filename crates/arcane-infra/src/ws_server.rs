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
//! All client/server framing is **postcard binary** via the [`arcane_wire`]
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
//! Each tick the producer serializes every entity **once** to a postcard byte
//! chunk, builds a [`PreEncodedTick`] holding `Arc<Vec<u8>>` chunks plus the
//! delta header and removed-id list, and broadcasts that to all subscribers.
//! Per-client tasks assemble their outbound frame by selecting which entity
//! chunks to include (today: all; with AOI: a filtered subset) and calling
//! [`arcane_wire::encode_server_delta_from_chunks`]. Subscribers never
//! re-serialize individual entities, so the per-tick cost is O(entity count)
//! at the producer and O(entity count × subscriber count) in Arc clones and
//! byte-concatenation — never O(entity count × subscriber count) in
//! serialization, which was the regression this design fixes.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use arcane_core::cluster_simulation::GameAction;
use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
use arcane_core::Vec3;
use arcane_wire::{
    ClientFrame, DeltaHeader, EntityState as WireEntityState, GameActionPayload,
    PlayerStatePayload, Vec3 as WireVec3,
};
use futures_util::{sink::SinkExt, stream::StreamExt};
use rayon::prelude::*;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::cluster_stats::ClusterStats;

/// Maximum byte length of a single inbound WebSocket binary payload. Client
/// frames larger than this are dropped and counted as parse failures —
/// defends against runaway allocations from misbehaving clients.
const MAX_MESSAGE_BYTES: usize = 64 * 1024; // 64 KiB

fn should_keep_ws_loop_running_on_broadcast_error(
    error: &tokio::sync::broadcast::error::RecvError,
) -> bool {
    matches!(error, tokio::sync::broadcast::error::RecvError::Lagged(_))
}

/// Convert a [`PlayerStatePayload`] (wire-side) into the cluster-internal
/// [`EntityStateEntry`]. `user_data` bytes are deserialized as JSON if
/// non-empty; empty bytes produce [`serde_json::Value::Null`]. `cluster_id`
/// is set to nil — the cluster binary applies its own when routing.
fn entry_from_wire_player_state(payload: &PlayerStatePayload) -> Option<EntityStateEntry> {
    if !payload.position.x.is_finite()
        || !payload.position.y.is_finite()
        || !payload.position.z.is_finite()
        || !payload.velocity.x.is_finite()
        || !payload.velocity.y.is_finite()
        || !payload.velocity.z.is_finite()
    {
        return None;
    }
    let user_data = if payload.user_data.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&payload.user_data).ok()?
    };
    let mut entry = EntityStateEntry::new(
        payload.entity_id,
        Uuid::nil(),
        Vec3::new(payload.position.x, payload.position.y, payload.position.z),
        Vec3::new(payload.velocity.x, payload.velocity.y, payload.velocity.z),
    );
    entry.user_data = user_data;
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

/// Convert one cluster-internal [`EntityStateEntry`] to a postcard-encoded
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
        // Shouldn't fail for any valid Value; log-and-drop is fine if it does.
        serde_json::to_vec(&entity.user_data).unwrap_or_default()
    };
    let wire = WireEntityState {
        entity_id: entity.entity_id,
        cluster_id: entity.cluster_id,
        position: WireVec3::new(entity.position.x, entity.position.y, entity.position.z),
        velocity: WireVec3::new(entity.velocity.x, entity.velocity.y, entity.velocity.z),
        user_data: user_data_bytes,
    };
    arcane_wire::encode_entity_state(&wire).unwrap_or_default()
}

/// Per-tick snapshot assembled once by the producer and broadcast to all
/// subscribers. Each per-client task builds its outbound wire frame by
/// selecting which entity chunks it wants (today: all; with AOI: a subset)
/// and calling [`arcane_wire::encode_server_delta_from_chunks`].
///
/// Entity chunks are `Arc<Vec<u8>>` so N subscribers share one allocation per
/// entity per tick — the whole point of this struct and of Shape B.
struct PreEncodedTick {
    header: DeltaHeader,
    entity_chunks: Vec<Arc<Vec<u8>>>,
    removed: Vec<Uuid>,
}

/// Convert an [`EntityStateDelta`] to a [`PreEncodedTick`] by serializing
/// each entity once. Called by the producer task every tick.
///
/// Entity encoding is **parallelized across rayon's thread pool** via
/// `par_iter`. At scale the per-tick encode is O(P) in total world entity
/// count (every cluster encodes every entity it broadcasts, including
/// remote entities replicated from neighbors under full-mesh). On an
/// 8-vCPU cluster node this work was the dominant serial bottleneck in
/// the tick budget — measured at ~35 ms/tick for P=6750. Distributing it
/// across cores cuts the wall-clock proportionally (memory-bandwidth-
/// bounded, so usually 4-6× on 8 cores, not 8×).
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
    PreEncodedTick {
        header: DeltaHeader {
            source_cluster_id: delta.source_cluster_id,
            seq: delta.seq,
            tick: delta.tick,
            timestamp: delta.timestamp,
        },
        entity_chunks,
        removed: delta.removed.clone(),
    }
}

/// Assemble the outbound wire frame for one subscriber from a shared
/// [`PreEncodedTick`]. Today every subscriber takes all chunks; with AOI it
/// will take a filtered subset.
fn assemble_outbound_frame(tick: &PreEncodedTick) -> Result<Vec<u8>, arcane_wire::Error> {
    let chunk_refs: Vec<&[u8]> = tick.entity_chunks.iter().map(|c| c.as_slice()).collect();
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
    stats: &ClusterStats,
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

pub fn run_ws_server(
    port: u16,
    state_rx: Receiver<EntityStateDelta>,
    client_updates_tx: Sender<EntityStateEntry>,
    game_actions_tx: Sender<GameAction>,
    stats: Arc<ClusterStats>,
) {
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

async fn ws_loop(
    port: u16,
    state_rx: Receiver<EntityStateDelta>,
    client_updates_tx: Sender<EntityStateEntry>,
    game_actions_tx: Sender<GameAction>,
    stats: Arc<ClusterStats>,
) {
    // Broadcast carries a PreEncodedTick — per-entity postcard chunks plus
    // delta header and removed-id list. Subscribers assemble their outbound
    // frame from the shared chunks via arcane_wire::encode_server_delta_from_chunks.
    // One serialization per entity per tick, shared across all subscribers.
    let (broadcast_tx, _) = tokio::sync::broadcast::channel::<Arc<PreEncodedTick>>(256);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await.expect("bind ws port");
    eprintln!(
        "cluster WebSocket listening on ws://{} (binary arcane-wire frames only)",
        addr
    );

    let broadcast_tx = Arc::new(broadcast_tx);
    let tx_clone = broadcast_tx.clone();
    let rx = Arc::new(std::sync::Mutex::new(state_rx));
    tokio::spawn(async move {
        loop {
            let r = rx.clone();
            let delta = tokio::task::spawn_blocking(move || r.lock().unwrap().recv())
                .await
                .unwrap();
            match delta {
                Ok(d) => {
                    // Per-tick producer work: serialize every entity once
                    // and build the shared tick struct. All subscribers
                    // reuse these Arc-shared chunks without re-serializing.
                    let tick = Arc::new(pre_encode_tick(&d));
                    let _ = tx_clone.send(tick);
                }
                Err(_) => break,
            }
        }
    });

    while let Ok((stream, peer_addr)) = listener.accept().await {
        let mut ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let accept_n = stats.ws_accepts.fetch_add(1, Ordering::Relaxed) + 1;
        if accept_n <= 3 || accept_n.is_power_of_two() {
            eprintln!("ws accept #{} from {}", accept_n, peer_addr);
        }
        let mut recv = broadcast_tx.subscribe();
        let updates_tx = client_updates_tx.clone();
        let actions_tx = game_actions_tx.clone();
        let stats = stats.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = recv.recv() => {
                        match result {
                            Ok(tick_arc) => {
                                // Subscriber-side work: pick entity chunks
                                // (today: all; with AOI: filtered subset)
                                // and assemble a wire-compatible frame. No
                                // per-entity re-serialization happens here.
                                let bytes = match assemble_outbound_frame(&tick_arc) {
                                    Ok(b) => b,
                                    Err(_) => continue,
                                };
                                let byte_len = bytes.len() as u64;
                                if ws_stream.send(Message::Binary(bytes)).await.is_err() {
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
                                let _ = handle_binary_client_frame(&bytes, &updates_tx, &actions_tx, &stats);
                            }
                            Some(Err(_)) | None => break,
                            // Text and other frame types are not part of the
                            // supported wire protocol and are silently ignored.
                            _ => {}
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assemble_outbound_frame, encode_entity_chunk, entry_from_wire_player_state,
        game_action_from_wire, pre_encode_tick, should_keep_ws_loop_running_on_broadcast_error,
    };
    use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
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

    // ── inbound wire decode (kept from the pre-Shape-B era) ──────────────

    #[test]
    fn entry_from_wire_rejects_non_finite_position() {
        let payload = PlayerStatePayload {
            entity_id: Uuid::from_u128(1),
            position: WireVec3::new(f64::INFINITY, 0.0, 0.0),
            velocity: WireVec3::new(0.0, 0.0, 0.0),
            user_data: Vec::new(),
        };
        assert!(entry_from_wire_player_state(&payload).is_none());
    }

    #[test]
    fn entry_from_wire_accepts_empty_user_data_as_null() {
        let id = Uuid::from_u128(42);
        let payload = PlayerStatePayload {
            entity_id: id,
            position: WireVec3::new(1.0, 2.0, 3.0),
            velocity: WireVec3::new(0.0, 0.1, 0.0),
            user_data: Vec::new(),
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
            position: WireVec3::new(0.0, 0.0, 0.0),
            velocity: WireVec3::new(0.0, 0.0, 0.0),
            user_data: serde_json::to_vec(&serde_json::json!({"hp": 99})).unwrap(),
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
        let id = Uuid::from_u128(123);
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: id,
            position: WireVec3::new(1.25, 2.5, 3.75),
            velocity: WireVec3::new(0.1, 0.0, -0.1),
            user_data: Vec::new(),
        });
        let bytes = arcane_wire::encode_client(&frame).unwrap();
        let decoded = arcane_wire::decode_client(&bytes).unwrap();
        let ClientFrame::PlayerState(payload) = decoded else {
            panic!("expected PlayerState variant");
        };
        let entry = entry_from_wire_player_state(&payload).expect("parse");
        assert_eq!(entry.entity_id, id);
        assert_eq!(entry.position.x, 1.25);
        assert_eq!(entry.velocity.z, -0.1);
    }

    // ── Shape B primitives: encode_entity_chunk + pre_encode_tick +
    //    assemble_outbound_frame ─────────────────────────────────────────

    /// A chunk produced by `encode_entity_chunk` must be a valid standalone
    /// postcard encoding of a `WireEntityState` — i.e. decode round-trip
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
        assert_eq!(wire.position.x, 1.0);
        assert_eq!(wire.velocity.z, 0.3);
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

        let bytes = assemble_outbound_frame(&pre_tick).expect("assemble");
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
        let bytes = assemble_outbound_frame(&pre_tick).expect("assemble");
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

        let via_shape_b = assemble_outbound_frame(&pre_encode_tick(&delta)).expect("shape-b bytes");

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
                    position: WireVec3::new(e.position.x, e.position.y, e.position.z),
                    velocity: WireVec3::new(e.velocity.x, e.velocity.y, e.velocity.z),
                    user_data,
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
        let via_reference =
            arcane_wire::encode_server(&ServerFrame::Delta(reference_payload)).expect("ref bytes");

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
        let bytes = assemble_outbound_frame(&pre_tick).expect("assemble");
        let decoded = arcane_wire::decode_server(&bytes).expect("decode");
        let ServerFrame::Delta(payload) = decoded;
        for (i, e) in payload.updated.iter().enumerate() {
            assert_eq!(e.entity_id, entities[i].entity_id);
        }
    }
}
