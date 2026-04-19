//! Stable wire schema + codec helpers for Arcane cluster ↔ client WebSocket
//! messages. Deliberately small and dependency-light so both the cluster
//! (`arcane-infra`) and external clients (swarm driver, UE5 adapter, any
//! downstream game using Arcane) can depend on it without pulling in a
//! server-shaped dependency tree.
//!
//! ## Scope
//!
//! - **Schema** — the Rust types that get serialized on the wire.
//! - **Codec** — encode/decode helpers. Uses [`postcard`] internally; if we
//!   ever swap codecs, callers do not need to change.
//!
//! ## Not in scope
//!
//! - Internal game types (positions, entities as the game sees them) — those
//!   live in [`arcane-core`].
//! - WebSocket server/client logic — that stays with the consumer.
//! - Connection management, routing, parsing dispatch.
//!
//! ## `user_data` handling
//!
//! Arcane's four-bucket state model keeps a JSON bucket (`user_data`) on each
//! entity for application-specific replicated state. On the wire we carry it
//! as an opaque `Vec<u8>` — the consumer decides the bytes' interpretation.
//! In practice today the bytes are `serde_json` output; tomorrow they might be
//! something else. Keeping it opaque at this layer means `arcane-wire` stays
//! decoupled from the JSON library of the moment, and postcard doesn't have
//! to know how to serialize [`serde_json::Value`] (which is awkward).
//!
//! ## Compatibility
//!
//! Wire types are treated as append-only with respect to enum variants and
//! struct fields. Adding a new enum variant is a breaking change for old
//! decoders — every codec bump must be considered explicit protocol version.
//! Today there is no version byte; add one when the first backwards-compat
//! concern appears.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 3D position / velocity. Wire-level primitive; mirrors the cluster-internal
/// `arcane_core::Vec3` but is declared here so this crate carries no
/// dependency on `arcane-core`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

/// One entity's replicated state (spine + opaque user_data bytes).
///
/// The wire layout intentionally omits the four-bucket model's `local_data`:
/// it is never meant to cross the replication boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityState {
    pub entity_id: Uuid,
    pub cluster_id: Uuid,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Opaque application-replicated state (typically JSON bytes). Empty
    /// vector means "no user_data set."
    pub user_data: Vec<u8>,
}

/// Snapshot of entity state updates + removals the cluster broadcasts to
/// clients each tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeltaPayload {
    pub source_cluster_id: Uuid,
    pub seq: i64,
    pub tick: u64,
    pub timestamp: f64,
    pub updated: Vec<EntityState>,
    pub removed: Vec<Uuid>,
}

/// `PLAYER_STATE`-equivalent: client pushing its own entity spine + optional
/// user_data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerStatePayload {
    pub entity_id: Uuid,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Empty = "no user_data."
    pub user_data: Vec<u8>,
}

/// `GAME_ACTION`-equivalent: client-invoked action that routes through the
/// cluster's action channel. `payload` is opaque application bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameActionPayload {
    pub entity_id: Uuid,
    pub action_type: String,
    pub payload: Vec<u8>,
}

/// One message from a client to the cluster. Enum variants are wire-order
/// stable: new variants append.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientFrame {
    PlayerState(PlayerStatePayload),
    Action(GameActionPayload),
}

/// One message from the cluster to a client. Enum variants are wire-order
/// stable: new variants append.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerFrame {
    Delta(DeltaPayload),
}

/// Encode a value as postcard bytes. Returns a newly allocated `Vec<u8>`.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(value)
}

/// Decode a postcard byte slice into a value.
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entity() -> EntityState {
        EntityState {
            entity_id: Uuid::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888),
            cluster_id: Uuid::from_u128(0xaaaa_bbbb_cccc_dddd_eeee_ffff_0000_1111),
            position: Vec3::new(1.5, 2.0, -3.25),
            velocity: Vec3::new(0.0, 0.1, 0.0),
            user_data: b"{\"hp\":42}".to_vec(),
        }
    }

    #[test]
    fn client_frame_player_state_roundtrip() {
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(7),
            position: Vec3::new(1.0, 2.0, 3.0),
            velocity: Vec3::new(0.5, 0.0, -0.5),
            user_data: Vec::new(),
        });
        let bytes = encode(&frame).unwrap();
        let back: ClientFrame = decode(&bytes).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn client_frame_action_roundtrip() {
        let frame = ClientFrame::Action(GameActionPayload {
            entity_id: Uuid::from_u128(9),
            action_type: "use_item".to_string(),
            payload: br#"{"item_type":5}"#.to_vec(),
        });
        let bytes = encode(&frame).unwrap();
        let back: ClientFrame = decode(&bytes).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn server_frame_delta_roundtrip() {
        let frame = ServerFrame::Delta(DeltaPayload {
            source_cluster_id: Uuid::nil(),
            seq: 42,
            tick: 100,
            timestamp: 12.5,
            updated: vec![sample_entity()],
            removed: vec![Uuid::from_u128(99)],
        });
        let bytes = encode(&frame).unwrap();
        let back: ServerFrame = decode(&bytes).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn user_data_empty_preserved() {
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::nil(),
            position: Vec3::new(0.0, 0.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, 0.0),
            user_data: Vec::new(),
        });
        let bytes = encode(&frame).unwrap();
        let back: ClientFrame = decode(&bytes).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn decode_rejects_truncated_bytes() {
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(1),
            position: Vec3::new(1.0, 2.0, 3.0),
            velocity: Vec3::new(0.0, 0.0, 0.0),
            user_data: Vec::new(),
        });
        let bytes = encode(&frame).unwrap();
        let truncated = &bytes[..bytes.len() - 1];
        let result: Result<ClientFrame, _> = decode(truncated);
        assert!(
            result.is_err(),
            "truncated postcard bytes should fail to decode"
        );
    }
}
