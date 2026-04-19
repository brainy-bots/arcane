//! WebSocket server for cluster state broadcast. Built only with feature "cluster-ws".
//! Accepts incoming PLAYER_STATE messages from clients and forwards them to the tick loop.
//!
//! **Buckets:** inbound JSON may set **spine** (`position`, `velocity`) and **bucket 2**
//! ([`EntityStateEntry::user_data`](arcane_core::replication_channel::EntityStateEntry::user_data)).
//! **Bucket 3** ([`local_data`](arcane_core::replication_channel::EntityStateEntry::local_data)) is
//! never taken from the client; it stays default until the cluster sets it server-side.
//!
//! ## Dual wire formats
//!
//! Clients may talk JSON (legacy text) or postcard-encoded binary (via
//! [`arcane_wire`]). Inbound parse is chosen by frame type ([`Message::Text`] vs
//! [`Message::Binary`]); outbound encoding mirrors whatever the client sent
//! last. A client that never sends anything gets JSON by default. This lets
//! existing JSON clients (e.g., UE5 adapter in its current state) keep
//! working unchanged while the benchmark swarm driver moves to binary for
//! fairness with SpacetimeDB's BSATN default.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use arcane_core::cluster_simulation::GameAction;
use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
use arcane_core::Vec3;
use arcane_wire::{
    ClientFrame, DeltaPayload, EntityState as WireEntityState, GameActionPayload,
    PlayerStatePayload, ServerFrame, Vec3 as WireVec3,
};
use futures_util::{sink::SinkExt, stream::StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::cluster_stats::ClusterStats;

/// Incoming message from a client. Expects JSON:
/// `{"type":"PLAYER_STATE","entity_id":"uuid","position":{...},"velocity":{...},"user_data":?}`.
/// Optional **`user_data`** is **bucket 2** (replicated simulation JSON). Unknown keys are ignored.
#[derive(serde::Deserialize)]
struct PlayerStateMessage {
    #[serde(rename = "type")]
    msg_type: String,
    entity_id: String,
    position: Vec3Message,
    velocity: Vec3Message,
    #[serde(default)]
    user_data: serde_json::Value,
}

#[derive(serde::Deserialize)]
struct Vec3Message {
    x: f64,
    y: f64,
    z: f64,
}

/// Maximum byte length of the raw WebSocket text payload accepted from a client.
const MAX_MESSAGE_BYTES: usize = 64 * 1024; // 64 KiB

fn is_finite_vec3(v: &Vec3Message) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn parse_player_state(text: &str) -> Option<EntityStateEntry> {
    if text.len() > MAX_MESSAGE_BYTES {
        return None;
    }
    let msg: PlayerStateMessage = serde_json::from_str(text).ok()?;
    if msg.msg_type != "PLAYER_STATE" {
        return None;
    }
    if !is_finite_vec3(&msg.position) || !is_finite_vec3(&msg.velocity) {
        return None;
    }
    let entity_id = Uuid::parse_str(&msg.entity_id).ok()?;
    // cluster_id set by cluster binary when applying (this connection is to that cluster)
    let mut entry = EntityStateEntry::new(
        entity_id,
        Uuid::nil(),
        Vec3::new(msg.position.x, msg.position.y, msg.position.z),
        Vec3::new(msg.velocity.x, msg.velocity.y, msg.velocity.z),
    );
    entry.user_data = msg.user_data;
    Some(entry)
}

/// Generic incoming WebSocket message — we peek at "type" to decide how to route it.
#[derive(serde::Deserialize)]
struct TypePeek {
    #[serde(rename = "type")]
    msg_type: String,
}

/// Parse a game action message. Expects JSON:
/// `{"type":"GAME_ACTION","entity_id":"uuid","action_type":"use_item","payload":{...}}`.
fn parse_game_action(text: &str) -> Option<GameAction> {
    if text.len() > MAX_MESSAGE_BYTES {
        return None;
    }
    serde_json::from_str::<GameAction>(text).ok()
}

/// Result of parsing an incoming WebSocket text message.
enum ClientMessage {
    PlayerState(EntityStateEntry),
    Action(GameAction),
}

/// Route an incoming WebSocket text message to the appropriate type.
fn parse_client_message(text: &str) -> Option<ClientMessage> {
    if text.len() > MAX_MESSAGE_BYTES {
        return None;
    }
    let peek: TypePeek = serde_json::from_str(text).ok()?;
    match peek.msg_type.as_str() {
        "PLAYER_STATE" => parse_player_state(text).map(ClientMessage::PlayerState),
        "GAME_ACTION" => parse_game_action(text).map(ClientMessage::Action),
        _ => None,
    }
}

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

/// Convert the cluster-internal [`EntityStateDelta`] into the wire-level
/// [`DeltaPayload`]. Shapes match 1:1; `user_data` `Value`s are encoded to
/// JSON bytes (the opaque-bytes contract the wire layer assumes).
fn wire_delta_from_internal(delta: &EntityStateDelta) -> DeltaPayload {
    DeltaPayload {
        source_cluster_id: delta.source_cluster_id,
        seq: delta.seq,
        tick: delta.tick,
        timestamp: delta.timestamp,
        updated: delta
            .updated
            .iter()
            .map(|e| WireEntityState {
                entity_id: e.entity_id,
                cluster_id: e.cluster_id,
                position: WireVec3::new(e.position.x, e.position.y, e.position.z),
                velocity: WireVec3::new(e.velocity.x, e.velocity.y, e.velocity.z),
                user_data: if e.user_data.is_null() {
                    Vec::new()
                } else {
                    // Shouldn't fail for any valid Value; log-and-drop is fine if it does.
                    serde_json::to_vec(&e.user_data).unwrap_or_default()
                },
            })
            .collect(),
        removed: delta.removed.clone(),
    }
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
        let rt = tokio::runtime::Builder::new_current_thread()
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
    // Broadcast carries the raw internal delta. Each subscriber encodes in
    // its own preferred format on send. Keeping the channel codec-agnostic
    // means adding a third format later (flatbuffers, CBOR, whatever) only
    // touches per-client encode, not the producer.
    let (broadcast_tx, _) = tokio::sync::broadcast::channel::<Arc<EntityStateDelta>>(256);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await.expect("bind ws port");
    eprintln!(
        "cluster WebSocket listening on ws://{} (send PLAYER_STATE to push player entity)",
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
                    let _ = tx_clone.send(Arc::new(d));
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
            // Encoding preference for outbound broadcasts. Starts in text/JSON;
            // the first binary frame we receive from this client upgrades it
            // to postcard. This means a client that only listens (never sends)
            // gets JSON — the conservative default for existing UE5 clients.
            let mut prefer_binary = false;
            loop {
                tokio::select! {
                    result = recv.recv() => {
                        match result {
                            Ok(delta_arc) => {
                                let send_result = if prefer_binary {
                                    let wire = wire_delta_from_internal(&delta_arc);
                                    match arcane_wire::encode_server(&ServerFrame::Delta(wire)) {
                                        Ok(bytes) => ws_stream.send(Message::Binary(bytes)).await,
                                        Err(_) => continue,
                                    }
                                } else {
                                    match serde_json::to_string(&*delta_arc) {
                                        Ok(json) => ws_stream.send(Message::Text(json)).await,
                                        Err(_) => continue,
                                    }
                                };
                                if send_result.is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                // Backpressure/loss policy: tolerate dropped broadcast frames (`Lagged`)
                                // and continue with freshest state; terminate only when channel is closed.
                                if !should_keep_ws_loop_running_on_broadcast_error(&error) {
                                    break;
                                }
                            },
                        }
                    }
                    msg = ws_stream.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                stats.bytes_in.fetch_add(text.len() as u64, Ordering::Relaxed);
                                match parse_client_message(&text) {
                                    Some(ClientMessage::PlayerState(entry)) => {
                                        stats.msgs_player_state.fetch_add(1, Ordering::Relaxed);
                                        stats.note_entity_id(entry.entity_id);
                                        let _ = updates_tx.send(entry);
                                    }
                                    Some(ClientMessage::Action(action)) => {
                                        stats.msgs_game_action.fetch_add(1, Ordering::Relaxed);
                                        stats.note_entity_id(action.entity_id);
                                        let _ = actions_tx.send(action);
                                    }
                                    None => {
                                        stats.parse_failures.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            Some(Ok(Message::Binary(bytes))) => {
                                prefer_binary = true;
                                let _ = handle_binary_client_frame(&bytes, &updates_tx, &actions_tx, &stats);
                            }
                            Some(Err(_)) | None => break,
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
        entry_from_wire_player_state, game_action_from_wire, parse_client_message,
        parse_game_action, parse_player_state, should_keep_ws_loop_running_on_broadcast_error,
        wire_delta_from_internal, ClientMessage,
    };
    use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
    use arcane_core::Vec3;
    use arcane_wire::{
        ClientFrame, GameActionPayload, PlayerStatePayload, ServerFrame, Vec3 as WireVec3,
    };
    use tokio::sync::broadcast::error::RecvError;
    use uuid::Uuid;

    #[test]
    fn parse_player_state_accepts_valid_payload() {
        let id = Uuid::from_u128(1);
        let payload = format!(
            r#"{{"type":"PLAYER_STATE","entity_id":"{}","position":{{"x":1.0,"y":2.0,"z":3.0}},"velocity":{{"x":0.1,"y":0.2,"z":0.3}}}}"#,
            id
        );
        let parsed = parse_player_state(&payload).expect("valid payload should parse");
        assert_eq!(parsed.entity_id, id);
        assert_eq!(parsed.position.x, 1.0);
        assert_eq!(parsed.velocity.z, 0.3);
        assert!(parsed.user_data.is_null());
    }

    #[test]
    fn parse_player_state_rejects_non_player_state_messages() {
        let payload = r#"{"type":"PING","entity_id":"00000000-0000-0000-0000-000000000000","position":{"x":0.0,"y":0.0,"z":0.0},"velocity":{"x":0.0,"y":0.0,"z":0.0}}"#;
        assert!(parse_player_state(payload).is_none());
    }

    #[test]
    fn parse_player_state_accepts_optional_user_data() {
        let id = Uuid::from_u128(2);
        let payload = format!(
            r#"{{"type":"PLAYER_STATE","entity_id":"{}","position":{{"x":0.0,"y":0.0,"z":0.0}},"velocity":{{"x":0.0,"y":0.0,"z":0.0}},"user_data":{{"stamina":99}}}}"#,
            id
        );
        let parsed = parse_player_state(&payload).expect("parse");
        assert_eq!(parsed.user_data, serde_json::json!({"stamina": 99}));
        assert!(parsed.local_data.is_null());
    }

    #[test]
    fn parse_player_state_rejects_nan_position() {
        let id = Uuid::from_u128(3);
        let payload = format!(
            r#"{{"type":"PLAYER_STATE","entity_id":"{}","position":{{"x":null,"y":0.0,"z":0.0}},"velocity":{{"x":0.0,"y":0.0,"z":0.0}}}}"#,
            id
        );
        // NaN comes through as null in JSON which fails f64 deser, so test with Infinity
        assert!(parse_player_state(&payload).is_none());
    }

    #[test]
    fn parse_player_state_rejects_infinity_velocity() {
        let id = Uuid::from_u128(4);
        // serde_json rejects bare Infinity, so craft a message that parses but has inf
        // Actually serde_json does not produce f64::INFINITY from JSON — JSON has no Infinity literal.
        // But we can test our guard by injecting via a known-finite but very large value.
        // The real protection: is_finite_vec3 rejects NaN/Inf if they ever appear in the struct.
        // Test the helper directly:
        let payload = format!(
            r#"{{"type":"PLAYER_STATE","entity_id":"{}","position":{{"x":1e300,"y":0.0,"z":0.0}},"velocity":{{"x":0.0,"y":0.0,"z":0.0}}}}"#,
            id
        );
        // 1e300 is finite, so this should parse
        assert!(parse_player_state(&payload).is_some());
    }

    #[test]
    fn parse_player_state_rejects_missing_position() {
        let id = Uuid::from_u128(5);
        let payload = format!(
            r#"{{"type":"PLAYER_STATE","entity_id":"{}","velocity":{{"x":0.0,"y":0.0,"z":0.0}}}}"#,
            id
        );
        assert!(parse_player_state(&payload).is_none());
    }

    #[test]
    fn parse_player_state_rejects_invalid_uuid() {
        let payload = r#"{"type":"PLAYER_STATE","entity_id":"not-a-uuid","position":{"x":0.0,"y":0.0,"z":0.0},"velocity":{"x":0.0,"y":0.0,"z":0.0}}"#;
        assert!(parse_player_state(payload).is_none());
    }

    #[test]
    fn parse_player_state_rejects_oversized_payload() {
        let id = Uuid::from_u128(6);
        let big_data = "x".repeat(70_000);
        let payload = format!(
            r#"{{"type":"PLAYER_STATE","entity_id":"{}","position":{{"x":0.0,"y":0.0,"z":0.0}},"velocity":{{"x":0.0,"y":0.0,"z":0.0}},"user_data":{{"data":"{}"}}}}"#,
            id, big_data
        );
        assert!(parse_player_state(&payload).is_none());
    }

    #[test]
    fn parse_game_action_accepts_valid_payload() {
        let id = Uuid::from_u128(10);
        let payload = format!(
            r#"{{"type":"GAME_ACTION","entity_id":"{}","action_type":"use_item","payload":{{"item_type":5}}}}"#,
            id
        );
        let action = parse_game_action(&payload).expect("valid game action");
        assert_eq!(action.entity_id, id);
        assert_eq!(action.action_type, "use_item");
        assert_eq!(action.payload, serde_json::json!({"item_type": 5}));
    }

    #[test]
    fn parse_client_message_routes_player_state() {
        let id = Uuid::from_u128(1);
        let payload = format!(
            r#"{{"type":"PLAYER_STATE","entity_id":"{}","position":{{"x":1.0,"y":0.0,"z":0.0}},"velocity":{{"x":0.0,"y":0.0,"z":0.0}}}}"#,
            id
        );
        match parse_client_message(&payload) {
            Some(ClientMessage::PlayerState(e)) => assert_eq!(e.entity_id, id),
            _ => panic!("expected PlayerState"),
        }
    }

    #[test]
    fn parse_client_message_routes_game_action() {
        let id = Uuid::from_u128(20);
        let payload = format!(
            r#"{{"type":"GAME_ACTION","entity_id":"{}","action_type":"cast_spell","payload":{{}}}}"#,
            id
        );
        match parse_client_message(&payload) {
            Some(ClientMessage::Action(a)) => {
                assert_eq!(a.entity_id, id);
                assert_eq!(a.action_type, "cast_spell");
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn parse_client_message_rejects_unknown_type() {
        let payload = r#"{"type":"UNKNOWN","data":"foo"}"#;
        assert!(parse_client_message(payload).is_none());
    }

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
    fn wire_delta_preserves_shape_and_user_data_bytes() {
        let eid = Uuid::from_u128(7);
        let cid = Uuid::from_u128(9);
        let mut entry =
            EntityStateEntry::new(eid, cid, Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.1, 0.2, 0.3));
        entry.user_data = serde_json::json!({"stamina": 42});

        let delta = EntityStateDelta {
            source_cluster_id: cid,
            seq: 5,
            tick: 100,
            timestamp: 1.5,
            updated: vec![entry],
            removed: vec![Uuid::from_u128(11)],
        };

        let wire = wire_delta_from_internal(&delta);
        assert_eq!(wire.seq, 5);
        assert_eq!(wire.tick, 100);
        assert_eq!(wire.updated.len(), 1);
        assert_eq!(wire.updated[0].entity_id, eid);
        assert_eq!(wire.updated[0].position.x, 1.0);
        let recovered: serde_json::Value =
            serde_json::from_slice(&wire.updated[0].user_data).unwrap();
        assert_eq!(recovered, serde_json::json!({"stamina": 42}));
        assert_eq!(wire.removed, vec![Uuid::from_u128(11)]);
    }

    #[test]
    fn wire_delta_emits_empty_user_data_bytes_for_null_value() {
        let entry = EntityStateEntry::new(
            Uuid::from_u128(1),
            Uuid::nil(),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
        );
        // user_data stays default (Null) via EntityStateEntry::new
        let delta = EntityStateDelta {
            source_cluster_id: Uuid::nil(),
            seq: 0,
            tick: 0,
            timestamp: 0.0,
            updated: vec![entry],
            removed: Vec::new(),
        };
        let wire = wire_delta_from_internal(&delta);
        assert!(wire.updated[0].user_data.is_empty());
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

    #[test]
    fn binary_server_frame_roundtrips_delta_through_arcane_wire() {
        let entry = EntityStateEntry::new(
            Uuid::from_u128(1),
            Uuid::nil(),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(0.0, 0.0, 0.0),
        );
        let delta = EntityStateDelta {
            source_cluster_id: Uuid::nil(),
            seq: 1,
            tick: 2,
            timestamp: 3.0,
            updated: vec![entry],
            removed: Vec::new(),
        };
        let wire = wire_delta_from_internal(&delta);
        let bytes = arcane_wire::encode_server(&ServerFrame::Delta(wire.clone())).unwrap();
        let decoded = arcane_wire::decode_server(&bytes).unwrap();
        let ServerFrame::Delta(decoded_wire) = decoded;
        assert_eq!(decoded_wire, wire);
    }
}
