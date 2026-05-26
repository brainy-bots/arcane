//! Stable wire schema + codec helpers for Arcane cluster <-> client WebSocket
//! messages. Uses FlatBuffers for schema-driven codegen: one `.fbs` schema
//! (`schema/arcane_wire.fbs`) generates correct codecs for every engine plugin
//! (C++, C#, Rust, etc.) via `flatc`.
//!
//! ## Decode: materialized types
//!
//! `decode_*` functions currently materialize into owned Rust types (copying
//! `user_data` bytes). This preserves API stability for consumers. If
//! profiling shows decode alloc/copy on the hot path, switch to the zero-copy
//! FlatBuffer accessors in the `fb` module directly.

#[allow(unused_imports, dead_code, clippy::all)]
#[path = "generated/arcane_wire_generated.rs"]
mod arcane_wire_generated;

/// FlatBuffers-generated types. Use these directly for zero-copy decode on
/// hot paths; most consumers should prefer the typed encode/decode functions
/// at the crate root.
pub mod fb {
    pub use crate::arcane_wire_generated::arcane_wire::*;
}

use flatbuffers::FlatBufferBuilder;
use uuid::Uuid;

// ── Error ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Error {
    InvalidBuffer(flatbuffers::InvalidFlatbuffer),
    UnknownPayloadVariant(&'static str, u8),
    ChecksumMismatch { expected: u32, actual: u32 },
    FrameTooShort,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidBuffer(e) => write!(f, "invalid FlatBuffer: {}", e),
            Error::UnknownPayloadVariant(union_name, tag) => {
                write!(f, "unknown {} variant: {}", union_name, tag)
            }
            Error::ChecksumMismatch { expected, actual } => {
                write!(
                    f,
                    "CRC32 checksum mismatch: expected {:#010x}, got {:#010x}",
                    expected, actual
                )
            }
            Error::FrameTooShort => write!(f, "frame too short for CRC32 checksum (< 4 bytes)"),
        }
    }
}

impl std::error::Error for Error {}

impl From<flatbuffers::InvalidFlatbuffer> for Error {
    fn from(e: flatbuffers::InvalidFlatbuffer) -> Self {
        Error::InvalidBuffer(e)
    }
}

// ── Vec3 / Vec3Q ───────────────────────────────────────────────────────────

/// 3D position / velocity in f64 — the in-process representation. Mirrors
/// `arcane_core::Vec3` so this crate stays decoupled from the core types.
///
/// **Not the on-wire type** — see [`Vec3Q`].
#[derive(Clone, Copy, Debug, PartialEq)]
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

/// 3D position / velocity quantized to `i16` (6 B per Vec3 on the wire).
/// Scale = 1.0 today: one i16 unit = one f64 unit. Saturation: out-of-range
/// f64 values clamp to i16 bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vec3Q {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

impl Vec3Q {
    pub const fn new(x: i16, y: i16, z: i16) -> Self {
        Self { x, y, z }
    }

    pub fn from_vec3(v: Vec3) -> Self {
        Self {
            x: quantize(v.x),
            y: quantize(v.y),
            z: quantize(v.z),
        }
    }

    pub fn to_vec3(self) -> Vec3 {
        Vec3 {
            x: self.x as f64,
            y: self.y as f64,
            z: self.z as f64,
        }
    }
}

#[inline]
fn quantize(v: f64) -> i16 {
    if v.is_nan() {
        return 0;
    }
    v.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

// ── Wire types (owned, for the public encode/decode API) ───────────────────

/// One entity's replicated state (spine + opaque user_data bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityState {
    pub entity_id: Uuid,
    pub cluster_id: Uuid,
    pub position: Vec3Q,
    pub velocity: Vec3Q,
    pub user_data: Vec<u8>,
    pub client_seq: u64,
}

/// Server -> client: entity updates + removals for one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct DeltaPayload {
    pub source_cluster_id: Uuid,
    pub seq: i64,
    pub tick: u64,
    pub timestamp: f64,
    pub updated: Vec<EntityState>,
    pub removed: Vec<Uuid>,
}

/// Client -> server: player pushing own entity spine + optional user_data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerStatePayload {
    pub entity_id: Uuid,
    pub position: Vec3Q,
    pub velocity: Vec3Q,
    pub user_data: Vec<u8>,
    pub client_seq: u64,
}

/// Client -> server: game action routed through the cluster's action channel.
#[derive(Clone, Debug, PartialEq)]
pub struct GameActionPayload {
    pub entity_id: Uuid,
    pub action_type: String,
    pub payload: Vec<u8>,
}

/// One message from a client to the cluster.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientFrame {
    PlayerState(PlayerStatePayload),
    Action(GameActionPayload),
}

/// One message from the cluster to a client.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerFrame {
    Delta(DeltaPayload),
}

/// Header fields for a [`DeltaPayload`] — everything except the entity and
/// removed lists. Used with [`encode_server_delta_from_chunks`].
#[derive(Clone, Debug, PartialEq)]
pub struct DeltaHeader {
    pub source_cluster_id: Uuid,
    pub seq: i64,
    pub tick: u64,
    pub timestamp: f64,
}

// ── UUID conversion ────────────────────────────────────────────────────────

fn uuid_to_fb(id: &Uuid) -> fb::UUID {
    let bytes = id.as_bytes();
    let lo = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let hi = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    fb::UUID::new(lo, hi)
}

fn uuid_from_fb(id: &fb::UUID) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&id.lo().to_le_bytes());
    bytes[8..].copy_from_slice(&id.hi().to_le_bytes());
    Uuid::from_bytes(bytes)
}

fn vec3q_to_fb(v: &Vec3Q) -> fb::Vec3Q {
    fb::Vec3Q::new(v.x, v.y, v.z)
}

fn vec3q_from_fb(v: Option<&fb::Vec3Q>) -> Vec3Q {
    match v {
        Some(v) => Vec3Q::new(v.x(), v.y(), v.z()),
        None => Vec3Q::new(0, 0, 0),
    }
}

// ── Encode ─────────────────────────────────────────────────────────────────

/// Encode a single [`EntityState`] as a standalone FlatBuffer. The server
/// pre-encodes each entity once per tick, then shares the bytes across all
/// subscribers via [`encode_server_delta_from_chunks`].
pub fn encode_entity_state(entity: &EntityState) -> Vec<u8> {
    let mut fbb = FlatBufferBuilder::with_capacity(128);

    let eid = uuid_to_fb(&entity.entity_id);
    let cid = uuid_to_fb(&entity.cluster_id);
    let pos = vec3q_to_fb(&entity.position);
    let vel = vec3q_to_fb(&entity.velocity);
    let ud = if entity.user_data.is_empty() {
        None
    } else {
        Some(fbb.create_vector(&entity.user_data))
    };

    let es = fb::EntityState::create(
        &mut fbb,
        &fb::EntityStateArgs {
            entity_id: Some(&eid),
            cluster_id: Some(&cid),
            position: Some(&pos),
            velocity: Some(&vel),
            user_data: ud,
            client_seq: entity.client_seq,
        },
    );

    fbb.finish_minimal(es);
    fbb.finished_data().to_vec()
}

/// Decode standalone FlatBuffer bytes back to an [`EntityState`].
pub fn decode_entity_state(bytes: &[u8]) -> Result<EntityState, Error> {
    let es = flatbuffers::root::<fb::EntityState>(bytes)?;
    Ok(entity_state_from_fb(&es))
}

fn entity_state_from_fb(es: &fb::EntityState) -> EntityState {
    EntityState {
        entity_id: uuid_from_fb(es.entity_id()),
        cluster_id: uuid_from_fb(es.cluster_id()),
        position: vec3q_from_fb(es.position()),
        velocity: vec3q_from_fb(es.velocity()),
        user_data: es
            .user_data()
            .map(|v| v.bytes().to_vec())
            .unwrap_or_default(),
        client_seq: es.client_seq(),
    }
}

/// Encode a [`ClientFrame`] as FlatBuffer bytes.
pub fn encode_client(frame: &ClientFrame) -> Vec<u8> {
    let mut fbb = FlatBufferBuilder::with_capacity(128);

    match frame {
        ClientFrame::PlayerState(ps) => {
            let eid = uuid_to_fb(&ps.entity_id);
            let pos = vec3q_to_fb(&ps.position);
            let vel = vec3q_to_fb(&ps.velocity);
            let ud = if ps.user_data.is_empty() {
                None
            } else {
                Some(fbb.create_vector(&ps.user_data))
            };

            let ps_fb = fb::PlayerStatePayload::create(
                &mut fbb,
                &fb::PlayerStatePayloadArgs {
                    entity_id: Some(&eid),
                    position: Some(&pos),
                    velocity: Some(&vel),
                    user_data: ud,
                    client_seq: ps.client_seq,
                },
            );

            let frame_fb = fb::ClientFrame::create(
                &mut fbb,
                &fb::ClientFrameArgs {
                    payload_type: fb::ClientPayload::PlayerState,
                    payload: Some(ps_fb.as_union_value()),
                },
            );
            fbb.finish_minimal(frame_fb);
        }
        ClientFrame::Action(action) => {
            let eid = uuid_to_fb(&action.entity_id);
            let action_type = fbb.create_string(&action.action_type);
            let payload = if action.payload.is_empty() {
                None
            } else {
                Some(fbb.create_vector(&action.payload))
            };

            let ga_fb = fb::GameActionPayload::create(
                &mut fbb,
                &fb::GameActionPayloadArgs {
                    entity_id: Some(&eid),
                    action_type: Some(action_type),
                    payload,
                },
            );

            let frame_fb = fb::ClientFrame::create(
                &mut fbb,
                &fb::ClientFrameArgs {
                    payload_type: fb::ClientPayload::Action,
                    payload: Some(ga_fb.as_union_value()),
                },
            );
            fbb.finish_minimal(frame_fb);
        }
    }

    fbb.finished_data().to_vec()
}

/// Decode FlatBuffer bytes into a [`ClientFrame`].
pub fn decode_client(bytes: &[u8]) -> Result<ClientFrame, Error> {
    let frame = flatbuffers::root::<fb::ClientFrame>(bytes)?;
    match frame.payload_type() {
        fb::ClientPayload::PlayerState => {
            let ps = frame.payload_as_player_state().expect("verified by root()");
            Ok(ClientFrame::PlayerState(PlayerStatePayload {
                entity_id: uuid_from_fb(ps.entity_id()),
                position: vec3q_from_fb(ps.position()),
                velocity: vec3q_from_fb(ps.velocity()),
                user_data: ps
                    .user_data()
                    .map(|v| v.bytes().to_vec())
                    .unwrap_or_default(),
                client_seq: ps.client_seq(),
            }))
        }
        fb::ClientPayload::Action => {
            let action = frame.payload_as_action().expect("verified by root()");
            Ok(ClientFrame::Action(GameActionPayload {
                entity_id: uuid_from_fb(action.entity_id()),
                action_type: action.action_type().to_string(),
                payload: action
                    .payload()
                    .map(|v| v.bytes().to_vec())
                    .unwrap_or_default(),
            }))
        }
        other => Err(Error::UnknownPayloadVariant("ClientPayload", other.0)),
    }
}

/// Encode a [`ServerFrame`] as FlatBuffer bytes. Implemented via
/// [`encode_entity_state`] + [`encode_server_delta_from_chunks`] so the
/// output is byte-identical regardless of which encode path the caller uses.
pub fn encode_server(frame: &ServerFrame) -> Vec<u8> {
    match frame {
        ServerFrame::Delta(delta) => {
            let chunks: Vec<Vec<u8>> = delta.updated.iter().map(encode_entity_state).collect();
            let chunk_refs: Vec<&[u8]> = chunks.iter().map(|c| c.as_slice()).collect();
            let header = DeltaHeader {
                source_cluster_id: delta.source_cluster_id,
                seq: delta.seq,
                tick: delta.tick,
                timestamp: delta.timestamp,
            };
            encode_server_delta_from_chunks(&header, &chunk_refs, &delta.removed)
        }
    }
}

/// Decode FlatBuffer bytes into a [`ServerFrame`].
pub fn decode_server(bytes: &[u8]) -> Result<ServerFrame, Error> {
    let frame = flatbuffers::root::<fb::ServerFrame>(bytes)?;
    match frame.payload_type() {
        fb::ServerPayload::Delta => {
            let delta = frame.payload_as_delta().expect("verified by root()");

            let mut updated = Vec::new();
            if let (Some(data), Some(offsets)) = (delta.entity_data(), delta.entity_offsets()) {
                let data = data.bytes();
                let len = offsets.len();
                for i in 0..len {
                    let start = offsets.get(i) as usize;
                    let end = if i + 1 < len {
                        offsets.get(i + 1) as usize
                    } else {
                        data.len()
                    };
                    let es = flatbuffers::root::<fb::EntityState>(&data[start..end])
                        .map_err(Error::InvalidBuffer)?;
                    updated.push(entity_state_from_fb(&es));
                }
            }

            let mut removed = Vec::new();
            if let Some(rem) = delta.removed() {
                for i in 0..rem.len() {
                    removed.push(uuid_from_fb(rem.get(i)));
                }
            }

            Ok(ServerFrame::Delta(DeltaPayload {
                source_cluster_id: uuid_from_fb(delta.source_cluster_id()),
                seq: delta.seq(),
                tick: delta.tick(),
                timestamp: delta.timestamp(),
                updated,
                removed,
            }))
        }
        other => Err(Error::UnknownPayloadVariant("ServerPayload", other.0)),
    }
}

/// Assemble a FlatBuffer-encoded `ServerFrame::Delta` from a header, a list
/// of already-encoded [`EntityState`] byte chunks, and removed entity ids.
///
/// This is the broadcast-fan-out primitive: serialize each entity **once** per
/// tick, then build per-subscriber frames by **concatenating** subsets of the
/// cached chunks. AOI filtering passes only the visible subset's chunks.
pub fn encode_server_delta_from_chunks(
    header: &DeltaHeader,
    entity_chunks: &[&[u8]],
    removed: &[Uuid],
) -> Vec<u8> {
    let mut fbb = FlatBufferBuilder::with_capacity(1024);

    let scid = uuid_to_fb(&header.source_cluster_id);

    // Concatenate entity chunks into a single byte vector with an offset index.
    let mut concat = Vec::new();
    let mut offsets = Vec::new();
    for chunk in entity_chunks {
        offsets.push(concat.len() as u32);
        concat.extend_from_slice(chunk);
    }

    let entity_data = if concat.is_empty() {
        None
    } else {
        Some(fbb.create_vector(&concat))
    };
    let entity_offsets = if offsets.is_empty() {
        None
    } else {
        Some(fbb.create_vector(&offsets))
    };

    let removed_uuids: Vec<fb::UUID> = removed.iter().map(uuid_to_fb).collect();
    let removed_vec = if removed_uuids.is_empty() {
        None
    } else {
        Some(fbb.create_vector(&removed_uuids))
    };

    let delta = fb::DeltaPayload::create(
        &mut fbb,
        &fb::DeltaPayloadArgs {
            source_cluster_id: Some(&scid),
            seq: header.seq,
            tick: header.tick,
            timestamp: header.timestamp,
            entity_data,
            entity_offsets,
            removed: removed_vec,
        },
    );

    let frame = fb::ServerFrame::create(
        &mut fbb,
        &fb::ServerFrameArgs {
            payload_type: fb::ServerPayload::Delta,
            payload: Some(delta.as_union_value()),
        },
    );

    fbb.finish_minimal(frame);
    fbb.finished_data().to_vec()
}

// ── CRC32 checksum wrapper ────────────────────────────────────────────────

/// Append a CRC32C checksum (4 bytes, little-endian) to `payload`.
/// Callers choose whether to use checked or unchecked encode/decode —
/// this is the checked path.
pub fn encode_with_checksum(payload: &[u8]) -> Vec<u8> {
    let crc = crc32fast::hash(payload);
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.extend_from_slice(payload);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// Validate and strip the trailing CRC32 checksum. Returns the payload
/// slice on success.
pub fn decode_with_checksum(bytes: &[u8]) -> Result<&[u8], Error> {
    if bytes.len() < 4 {
        return Err(Error::FrameTooShort);
    }
    let (payload, checksum_bytes) = bytes.split_at(bytes.len() - 4);
    let expected = u32::from_le_bytes(checksum_bytes.try_into().unwrap());
    let actual = crc32fast::hash(payload);
    if expected != actual {
        return Err(Error::ChecksumMismatch { expected, actual });
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q3(x: f64, y: f64, z: f64) -> Vec3Q {
        Vec3Q::from_vec3(Vec3::new(x, y, z))
    }

    fn sample_entity() -> EntityState {
        EntityState {
            entity_id: Uuid::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888),
            cluster_id: Uuid::from_u128(0xaaaa_bbbb_cccc_dddd_eeee_ffff_0000_1111),
            position: q3(1.5, 2.0, -3.25),
            velocity: q3(0.0, 0.1, 0.0),
            user_data: b"{\"hp\":42}".to_vec(),
            client_seq: 0xCAFE_BABE_DEAD_BEEF,
        }
    }

    #[test]
    fn client_frame_player_state_roundtrip() {
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(7),
            position: q3(1.0, 2.0, 3.0),
            velocity: q3(0.5, 0.0, -0.5),
            user_data: Vec::new(),
            client_seq: 0,
        });
        let bytes = encode_client(&frame);
        let back = decode_client(&bytes).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn client_seq_nonzero_roundtrip_client_frame() {
        let seq = 0xDEAD_BEEF_1234_5678_u64;
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(42),
            position: q3(10.0, 20.0, 30.0),
            velocity: q3(1.0, 0.0, -1.0),
            user_data: Vec::new(),
            client_seq: seq,
        });
        let bytes = encode_client(&frame);
        let back = decode_client(&bytes).unwrap();
        let ClientFrame::PlayerState(p) = back else {
            panic!("expected PlayerState");
        };
        assert_eq!(p.client_seq, seq);
    }

    #[test]
    fn client_seq_nonzero_roundtrip_entity_state() {
        let e = sample_entity();
        assert_ne!(e.client_seq, 0, "sample_entity must use non-zero client_seq");
        let bytes = encode_entity_state(&e);
        let back = decode_entity_state(&bytes).unwrap();
        assert_eq!(back.client_seq, e.client_seq);
    }

    #[test]
    fn client_seq_nonzero_roundtrip_server_delta() {
        let e = sample_entity();
        let frame = ServerFrame::Delta(DeltaPayload {
            source_cluster_id: Uuid::nil(),
            seq: 1,
            tick: 1,
            timestamp: 0.0,
            updated: vec![e.clone()],
            removed: vec![],
        });
        let bytes = encode_server(&frame);
        let back = decode_server(&bytes).unwrap();
        let ServerFrame::Delta(payload) = back;
        assert_eq!(payload.updated[0].client_seq, e.client_seq);
    }

    #[test]
    fn client_frame_action_roundtrip() {
        let frame = ClientFrame::Action(GameActionPayload {
            entity_id: Uuid::from_u128(9),
            action_type: "use_item".to_string(),
            payload: br#"{"item_type":5}"#.to_vec(),
        });
        let bytes = encode_client(&frame);
        let back = decode_client(&bytes).unwrap();
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
        let bytes = encode_server(&frame);
        let back = decode_server(&bytes).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn user_data_empty_preserved() {
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::nil(),
            position: q3(0.0, 0.0, 0.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
            client_seq: 0,
        });
        let bytes = encode_client(&frame);
        let back = decode_client(&bytes).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn user_data_as_json_bytes_roundtrips_end_to_end() {
        let original_value = serde_json::json!({
            "hp": 42,
            "buffs": ["haste", "shield"],
            "pos": {"x": 1.5, "y": -3.0},
        });
        let json_bytes = serde_json::to_vec(&original_value).unwrap();

        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(7),
            position: q3(1.0, 2.0, 3.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: json_bytes,
            client_seq: 0,
        });

        let wire_bytes = encode_client(&frame);
        let decoded = decode_client(&wire_bytes).unwrap();

        let ClientFrame::PlayerState(payload) = decoded else {
            panic!("expected PlayerState variant");
        };
        let recovered: serde_json::Value = serde_json::from_slice(&payload.user_data).unwrap();
        assert_eq!(recovered, original_value);
    }

    // ── encode_server_delta_from_chunks ──

    fn sample_header() -> DeltaHeader {
        DeltaHeader {
            source_cluster_id: Uuid::from_u128(0xcafe_babe_dead_beef_0000_1111_2222_3333),
            seq: 42,
            tick: 100,
            timestamp: 12.5,
        }
    }

    /// Chunk-assembled output is byte-identical to `encode_server` output.
    /// True by construction: `encode_server` delegates to
    /// `encode_server_delta_from_chunks`.
    #[test]
    fn chunk_assembled_frame_matches_full_encode() {
        let e1 = sample_entity();
        let e2 = EntityState {
            entity_id: Uuid::from_u128(7),
            cluster_id: Uuid::from_u128(9),
            position: q3(0.0, 0.0, 0.0),
            velocity: q3(0.25, -0.5, 0.75),
            user_data: Vec::new(),
            client_seq: 0,
        };
        let removed = vec![Uuid::from_u128(0xdead), Uuid::from_u128(0xbeef)];

        let header = sample_header();
        let full_payload = DeltaPayload {
            source_cluster_id: header.source_cluster_id,
            seq: header.seq,
            tick: header.tick,
            timestamp: header.timestamp,
            updated: vec![e1.clone(), e2.clone()],
            removed: removed.clone(),
        };
        let via_full = encode_server(&ServerFrame::Delta(full_payload));

        let chunk1 = encode_entity_state(&e1);
        let chunk2 = encode_entity_state(&e2);
        let via_chunks = encode_server_delta_from_chunks(
            &header,
            &[chunk1.as_slice(), chunk2.as_slice()],
            &removed,
        );

        assert_eq!(
            via_full, via_chunks,
            "chunk-assembled frame must be byte-identical to full encode"
        );
    }

    #[test]
    fn chunk_assembled_frame_with_empty_lists() {
        let header = sample_header();
        let via_chunks = encode_server_delta_from_chunks(&header, &[], &[]);

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.source_cluster_id, header.source_cluster_id);
        assert_eq!(payload.seq, header.seq);
        assert_eq!(payload.tick, header.tick);
        assert_eq!(payload.timestamp, header.timestamp);
        assert!(payload.updated.is_empty());
        assert!(payload.removed.is_empty());
    }

    #[test]
    fn chunk_assembled_frame_with_single_entity() {
        let header = sample_header();
        let e1 = sample_entity();
        let chunk1 = encode_entity_state(&e1);

        let via_chunks = encode_server_delta_from_chunks(&header, &[chunk1.as_slice()], &[]);

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.updated.len(), 1);
        assert_eq!(payload.updated[0], e1);
        assert!(payload.removed.is_empty());
    }

    /// Larger entity count (> 127) — verifies scale with fixed-width u32
    /// offsets.
    #[test]
    fn chunk_assembled_frame_with_large_entity_count() {
        let header = sample_header();
        let mut entities = Vec::new();
        let mut chunks = Vec::new();
        for i in 0..200_u128 {
            let e = EntityState {
                entity_id: Uuid::from_u128(i),
                cluster_id: Uuid::nil(),
                position: q3(i as f64, 0.0, 0.0),
                velocity: q3(0.0, 0.0, 0.0),
                user_data: Vec::new(),
                client_seq: 0,
            };
            chunks.push(encode_entity_state(&e));
            entities.push(e);
        }
        let chunk_refs: Vec<&[u8]> = chunks.iter().map(|c| c.as_slice()).collect();

        let via_chunks = encode_server_delta_from_chunks(&header, &chunk_refs, &[]);

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.updated.len(), 200);
        assert_eq!(payload.updated[150], entities[150]);

        let via_full = encode_server(&ServerFrame::Delta(DeltaPayload {
            source_cluster_id: header.source_cluster_id,
            seq: header.seq,
            tick: header.tick,
            timestamp: header.timestamp,
            updated: entities,
            removed: Vec::new(),
        }));

        assert_eq!(via_chunks, via_full);
    }

    #[test]
    fn chunk_assembled_frame_supports_subset_selection() {
        let header = sample_header();
        let e1 = EntityState {
            entity_id: Uuid::from_u128(1),
            cluster_id: Uuid::nil(),
            position: q3(1.0, 0.0, 0.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
            client_seq: 0,
        };
        let e2 = EntityState {
            entity_id: Uuid::from_u128(2),
            cluster_id: Uuid::nil(),
            position: q3(2.0, 0.0, 0.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
            client_seq: 0,
        };
        let e3 = EntityState {
            entity_id: Uuid::from_u128(3),
            cluster_id: Uuid::nil(),
            position: q3(3.0, 0.0, 0.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
            client_seq: 0,
        };
        let c1 = encode_entity_state(&e1);
        let c2 = encode_entity_state(&e2);
        let c3 = encode_entity_state(&e3);

        let via_chunks =
            encode_server_delta_from_chunks(&header, &[c1.as_slice(), c3.as_slice()], &[]);

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.updated.len(), 2);
        assert_eq!(payload.updated[0], e1);
        assert_eq!(payload.updated[1], e3);

        assert!(!c2.is_empty(), "unused chunks remain usable");
    }

    #[test]
    fn chunk_assembled_frame_preserves_user_data_bytes() {
        let header = sample_header();
        let e = EntityState {
            entity_id: Uuid::from_u128(42),
            cluster_id: Uuid::from_u128(99),
            position: q3(1.0, 2.0, 3.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: br#"{"hp":99,"status":"poisoned"}"#.to_vec(),
            client_seq: 0,
        };
        let chunk = encode_entity_state(&e);

        let via_chunks = encode_server_delta_from_chunks(&header, &[chunk.as_slice()], &[]);

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.updated.len(), 1);
        assert_eq!(payload.updated[0].user_data, e.user_data);
    }

    // ── Vec3Q ──

    #[test]
    fn vec3q_roundtrip_within_quantization_step() {
        let v = Vec3::new(1.4, 2.6, -3.5);
        let q = Vec3Q::from_vec3(v);
        let back = q.to_vec3();
        assert!((back.x - 1.0).abs() <= 0.5);
        assert!((back.y - 3.0).abs() <= 0.5);
        assert!((back.z - -4.0).abs() <= 0.5);
    }

    #[test]
    fn vec3q_saturates_above_i16_max() {
        let q = Vec3Q::from_vec3(Vec3::new(1e9, 0.0, 0.0));
        assert_eq!(q.x, i16::MAX);
    }

    #[test]
    fn vec3q_saturates_below_i16_min() {
        let q = Vec3Q::from_vec3(Vec3::new(-1e9, 0.0, 0.0));
        assert_eq!(q.x, i16::MIN);
    }

    #[test]
    fn vec3q_nan_becomes_zero() {
        let q = Vec3Q::from_vec3(Vec3::new(f64::NAN, f64::NAN, f64::NAN));
        assert_eq!(q, Vec3Q::new(0, 0, 0));
    }

    #[test]
    fn vec3q_infinity_clamps_to_bounds() {
        let q_pos = Vec3Q::from_vec3(Vec3::new(f64::INFINITY, 0.0, 0.0));
        let q_neg = Vec3Q::from_vec3(Vec3::new(f64::NEG_INFINITY, 0.0, 0.0));
        assert_eq!(q_pos.x, i16::MAX);
        assert_eq!(q_neg.x, i16::MIN);
    }

    #[test]
    fn decode_rejects_truncated_bytes() {
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(1),
            position: q3(1.0, 2.0, 3.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
            client_seq: 0,
        });
        let bytes = encode_client(&frame);
        let truncated = &bytes[..bytes.len() - 1];
        let result = decode_client(truncated);
        assert!(result.is_err(), "truncated bytes should fail to decode");
    }

    /// Entity state standalone encode/decode roundtrip.
    #[test]
    fn entity_state_roundtrip() {
        let entity = sample_entity();
        let bytes = encode_entity_state(&entity);
        let back = decode_entity_state(&bytes).unwrap();
        assert_eq!(entity, back);
    }

    /// UUID roundtrip through FlatBuffer struct encoding.
    #[test]
    fn uuid_roundtrip_through_fb() {
        let original = Uuid::from_u128(0x0123_4567_89ab_cdef_fedc_ba98_7654_3210);
        let fb_uuid = uuid_to_fb(&original);
        let back = uuid_from_fb(&fb_uuid);
        assert_eq!(original, back);
    }

    /// Nil UUID roundtrip.
    #[test]
    fn uuid_nil_roundtrip() {
        let nil = Uuid::nil();
        let fb_uuid = uuid_to_fb(&nil);
        let back = uuid_from_fb(&fb_uuid);
        assert_eq!(nil, back);
    }

    // ── CRC32 checksum ──

    #[test]
    fn checksum_roundtrip() {
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(7),
            position: q3(1.0, 2.0, 3.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
            client_seq: 0,
        });
        let payload = encode_client(&frame);
        let checked = encode_with_checksum(&payload);
        assert_eq!(checked.len(), payload.len() + 4);
        let stripped = decode_with_checksum(&checked).unwrap();
        assert_eq!(stripped, payload.as_slice());
        let back = decode_client(stripped).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn checksum_rejects_flipped_bit() {
        let payload = encode_client(&ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(1),
            position: q3(0.0, 0.0, 0.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: b"hello".to_vec(),
            client_seq: 0,
        }));
        let mut checked = encode_with_checksum(&payload);
        checked[5] ^= 0x01;
        let result = decode_with_checksum(&checked);
        assert!(
            matches!(result, Err(Error::ChecksumMismatch { .. })),
            "flipped bit must cause checksum mismatch"
        );
    }

    #[test]
    fn checksum_rejects_too_short() {
        assert!(matches!(
            decode_with_checksum(&[0, 1, 2]),
            Err(Error::FrameTooShort)
        ));
    }

    #[test]
    fn checksum_empty_payload() {
        let checked = encode_with_checksum(&[]);
        assert_eq!(checked.len(), 4);
        let stripped = decode_with_checksum(&checked).unwrap();
        assert!(stripped.is_empty());
    }
}
