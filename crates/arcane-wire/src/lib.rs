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

/// 3D position / velocity in continuous f64 — the in-process representation
/// the cluster simulation uses. Mirrors `arcane_core::Vec3` so this crate
/// stays decoupled from the core types.
///
/// **Not the on-wire type for entities** — see [`Vec3Q`]. `Vec3` is exposed
/// so callers (sim code, tests, conversions at the wire boundary) keep
/// working in f64; quantization happens at the wire boundary via
/// [`Vec3Q::from_vec3`].
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

/// 3D position / velocity quantized to `i16` (24 B → 6 B per Vec3 on the
/// wire). Used for [`EntityState`] and [`PlayerStatePayload`] fields where
/// per-entity bandwidth dominates the cluster outbound NIC bottleneck.
///
/// **Scale = 1.0** today: one i16 unit = one f64 unit. The benchmark world
/// is `0..5000` units, well inside i16's `±32_767` range with massive
/// safety margin. Loses sub-unit precision (1 unit = 2% of `COLLISION_RADIUS=50`,
/// well below the kinematic sim's noise floor).
///
/// If a future game needs sub-unit precision, scale is the easiest knob to
/// add — convention is to track it as a const generic or a wire-format
/// version field. Today it's hard-coded to keep the change minimal.
///
/// **Saturation behavior:** out-of-range f64 values clamp to i16 bounds
/// rather than wrap, so a runaway entity stays at the world edge instead of
/// teleporting across the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vec3Q {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

impl Vec3Q {
    pub const fn new(x: i16, y: i16, z: i16) -> Self {
        Self { x, y, z }
    }

    /// Quantize a continuous Vec3 (f64) to i16. NaN coerces to 0; out-of-
    /// range values saturate at i16 bounds.
    pub fn from_vec3(v: Vec3) -> Self {
        Self {
            x: quantize(v.x),
            y: quantize(v.y),
            z: quantize(v.z),
        }
    }

    /// Dequantize back to f64. The conversion is lossless on the i16 itself
    /// but the original f64's sub-unit precision was discarded at quantize
    /// time.
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

/// One entity's replicated state (spine + opaque user_data bytes).
///
/// `position` and `velocity` are quantized — see [`Vec3Q`] for the
/// scale + range tradeoff.
///
/// The wire layout intentionally omits the four-bucket model's `local_data`:
/// it is never meant to cross the replication boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityState {
    pub entity_id: Uuid,
    pub cluster_id: Uuid,
    pub position: Vec3Q,
    pub velocity: Vec3Q,
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
/// user_data. Position/velocity quantized — see [`Vec3Q`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerStatePayload {
    pub entity_id: Uuid,
    pub position: Vec3Q,
    pub velocity: Vec3Q,
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

/// Encode a `ClientFrame` as postcard bytes.
///
/// Typed rather than generic on purpose: callers should not be able to
/// serialize arbitrary types through this crate — only the wire-contract
/// frame types. This keeps the "what goes on the wire" story monotonic.
pub fn encode_client(frame: &ClientFrame) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(frame)
}

/// Decode a postcard byte slice into a `ClientFrame`.
pub fn decode_client(bytes: &[u8]) -> Result<ClientFrame, postcard::Error> {
    postcard::from_bytes(bytes)
}

/// Encode a `ServerFrame` as postcard bytes. Typed for the same reason as
/// [`encode_client`].
pub fn encode_server(frame: &ServerFrame) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(frame)
}

/// Decode a postcard byte slice into a `ServerFrame`.
pub fn decode_server(bytes: &[u8]) -> Result<ServerFrame, postcard::Error> {
    postcard::from_bytes(bytes)
}

/// Re-exported codec error type. Callers import this instead of `postcard::Error`
/// so they don't depend on the codec choice directly — swapping codecs later
/// changes only this crate's internals.
pub type Error = postcard::Error;

/// Encode a single [`EntityState`] as standalone postcard bytes. The intended
/// caller is the cluster's WS server, which pre-encodes each entity once per
/// tick and then shares the bytes across all subscribers via
/// [`encode_server_delta_from_chunks`]. Matches the per-entity byte layout
/// that would otherwise appear inside a full `ServerFrame::Delta` encoding.
pub fn encode_entity_state(entity: &EntityState) -> Result<Vec<u8>, Error> {
    postcard::to_allocvec(entity)
}

/// Decode standalone postcard bytes back to an [`EntityState`]. Mostly useful
/// in tests that validate the chunk-assembly primitives.
pub fn decode_entity_state(bytes: &[u8]) -> Result<EntityState, Error> {
    postcard::from_bytes(bytes)
}

/// Header fields for a [`DeltaPayload`] — everything except the `updated` and
/// `removed` lists. Used with [`encode_server_delta_from_chunks`] to assemble
/// a wire frame from pre-encoded per-entity bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct DeltaHeader {
    pub source_cluster_id: Uuid,
    pub seq: i64,
    pub tick: u64,
    pub timestamp: f64,
}

/// Assemble a postcard-encoded `ServerFrame::Delta` from a header, a list of
/// already-postcard-encoded [`EntityState`] byte chunks, and a list of removed
/// entity ids.
///
/// This is the broadcast-fan-out primitive the cluster server uses to
/// serialize each entity **once** per tick (at the producer) and then build
/// per-subscriber frames by **concatenating** the cached chunks — rather than
/// re-serializing the full delta N times, once per subscriber, which is the
/// O(N) cost pattern this primitive exists to avoid. It is also the hook for
/// AOI (area-of-interest) filtering: when a subscriber only wants a subset of
/// entities, pass only that subset's chunks; no re-serialization is needed.
///
/// ## Wire compatibility
///
/// The output is bit-for-bit identical to what
/// `encode_server(&ServerFrame::Delta(DeltaPayload { ... }))` would produce
/// for the equivalent fully-materialized payload. Existing clients decode the
/// output via the standard [`decode_server`]; they do not know (and do not
/// need to know) that the frame was assembled from chunks on the server side.
///
/// ## Input contract
///
/// Each byte slice in `entity_chunks` must be a valid postcard encoding of an
/// [`EntityState`] — typically produced by `postcard::to_allocvec(&entity)`
/// (or the equivalent on a `&EntityState`). Passing malformed bytes here
/// produces a malformed frame that downstream decoders will reject; this
/// function does not validate the chunks.
pub fn encode_server_delta_from_chunks(
    header: &DeltaHeader,
    entity_chunks: &[&[u8]],
    removed: &[Uuid],
) -> Result<Vec<u8>, postcard::Error> {
    // Strategy: serialize a scaffold `ServerFrame::Delta(DeltaPayload { ... })`
    // with empty `updated` and empty `removed`. The scaffold's last two bytes
    // are the postcard varints for those two empty-list lengths (each encodes
    // to a single 0 byte). We then replace that tail with (a) the varint for
    // our real updated count + the concatenated entity chunks, and (b) the
    // varint for our real removed count + the 16-byte UUID bytes for each
    // removed id.
    //
    // This avoids hand-rolling postcard's varint or enum-variant encoding —
    // we let postcard produce the header bytes for us, then splice in the two
    // list bodies.
    let scaffold_payload = DeltaPayload {
        source_cluster_id: header.source_cluster_id,
        seq: header.seq,
        tick: header.tick,
        timestamp: header.timestamp,
        updated: Vec::new(),
        removed: Vec::new(),
    };
    let scaffold = postcard::to_allocvec(&ServerFrame::Delta(scaffold_payload))?;

    // The scaffold encoding ends with two 0-byte varints (the two empty-list
    // lengths). Everything before them is the variant tag + fixed-width and
    // variable-width header fields we want verbatim.
    let header_len = scaffold
        .len()
        .checked_sub(2)
        .expect("scaffold always has two trailing zero-length varints");
    let mut out = Vec::with_capacity(header_len + chunks_total_len(entity_chunks) + 16);
    out.extend_from_slice(&scaffold[..header_len]);

    // Postcard's `Vec<T>` length prefix is the same varint encoding postcard
    // uses for any sequence — including `Vec<()>`. Serializing `Vec<()>` of
    // the right length is the cheapest way to emit exactly that varint
    // without reaching into postcard's internal varint helpers.
    let updated_len_prefix: Vec<u8> = postcard::to_allocvec(&vec![(); entity_chunks.len()])?;
    out.extend_from_slice(&updated_len_prefix);
    for chunk in entity_chunks {
        out.extend_from_slice(chunk);
    }

    // Removed list: varint(len) + 16 bytes per Uuid. Uuid's postcard encoding
    // is its 16 raw bytes in big-endian order. Using `postcard::to_allocvec`
    // on the whole `Vec<Uuid>` produces exactly that layout, so we just
    // serialize the list directly.
    let removed_bytes: Vec<u8> = postcard::to_allocvec(&removed.to_vec())?;
    out.extend_from_slice(&removed_bytes);

    Ok(out)
}

fn chunks_total_len(chunks: &[&[u8]]) -> usize {
    chunks.iter().map(|c| c.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-side shorthand: build a Vec3Q from f64 components via the public
    /// quantize path. Lets the test corpus continue to read in real-world
    /// units (e.g. `q3(1.5, 2.0, -3.25)`) instead of pre-rounded i16 literals.
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
        }
    }

    #[test]
    fn client_frame_player_state_roundtrip() {
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(7),
            position: q3(1.0, 2.0, 3.0),
            velocity: q3(0.5, 0.0, -0.5),
            user_data: Vec::new(),
        });
        let bytes = encode_client(&frame).unwrap();
        let back = decode_client(&bytes).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn client_frame_action_roundtrip() {
        let frame = ClientFrame::Action(GameActionPayload {
            entity_id: Uuid::from_u128(9),
            action_type: "use_item".to_string(),
            payload: br#"{"item_type":5}"#.to_vec(),
        });
        let bytes = encode_client(&frame).unwrap();
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
        let bytes = encode_server(&frame).unwrap();
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
        });
        let bytes = encode_client(&frame).unwrap();
        let back = decode_client(&bytes).unwrap();
        assert_eq!(frame, back);
    }

    /// The intended real-world use of `user_data`: carry JSON bytes produced
    /// by `serde_json::to_vec`, round-trip them through postcard, and
    /// deserialize back on the other side. Validates that the opaque-bytes
    /// design actually works for its primary use case.
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
        });

        let wire_bytes = encode_client(&frame).unwrap();
        let decoded = decode_client(&wire_bytes).unwrap();

        let ClientFrame::PlayerState(payload) = decoded else {
            panic!("expected PlayerState variant");
        };
        let recovered: serde_json::Value = serde_json::from_slice(&payload.user_data).unwrap();
        assert_eq!(recovered, original_value);
    }

    // ── encode_server_delta_from_chunks — Shape B producer primitive ──

    fn sample_header() -> DeltaHeader {
        DeltaHeader {
            source_cluster_id: Uuid::from_u128(0xcafe_babe_dead_beef_0000_1111_2222_3333),
            seq: 42,
            tick: 100,
            timestamp: 12.5,
        }
    }

    fn encode_entity(entity: &EntityState) -> Vec<u8> {
        postcard::to_allocvec(entity).expect("encode entity")
    }

    /// The key correctness property: chunk-assembled output is bit-for-bit
    /// identical to what `encode_server` produces for the equivalent fully-
    /// materialized payload. Existing decoders must not be able to tell the
    /// difference.
    #[test]
    fn chunk_assembled_frame_matches_full_encode_byte_for_byte() {
        let e1 = sample_entity();
        let e2 = EntityState {
            entity_id: Uuid::from_u128(7),
            cluster_id: Uuid::from_u128(9),
            position: q3(0.0, 0.0, 0.0),
            velocity: q3(0.25, -0.5, 0.75),
            user_data: Vec::new(),
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
        let via_full = encode_server(&ServerFrame::Delta(full_payload)).unwrap();

        let chunk1 = encode_entity(&e1);
        let chunk2 = encode_entity(&e2);
        let via_chunks = encode_server_delta_from_chunks(
            &header,
            &[chunk1.as_slice(), chunk2.as_slice()],
            &removed,
        )
        .unwrap();

        assert_eq!(
            via_full, via_chunks,
            "chunk-assembled frame must be byte-identical to full encode"
        );
    }

    /// Empty updated list and empty removed list — the degenerate case. Still
    /// has to be a valid frame that decodes correctly.
    #[test]
    fn chunk_assembled_frame_with_empty_lists() {
        let header = sample_header();
        let via_chunks = encode_server_delta_from_chunks(&header, &[], &[]).unwrap();

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.source_cluster_id, header.source_cluster_id);
        assert_eq!(payload.seq, header.seq);
        assert_eq!(payload.tick, header.tick);
        assert_eq!(payload.timestamp, header.timestamp);
        assert!(payload.updated.is_empty());
        assert!(payload.removed.is_empty());
    }

    /// Single entity, empty removed list — verifies the varint-prefix machinery
    /// around a single chunk is correct.
    #[test]
    fn chunk_assembled_frame_with_single_entity() {
        let header = sample_header();
        let e1 = sample_entity();
        let chunk1 = encode_entity(&e1);

        let via_chunks =
            encode_server_delta_from_chunks(&header, &[chunk1.as_slice()], &[]).unwrap();

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.updated.len(), 1);
        assert_eq!(payload.updated[0], e1);
        assert!(payload.removed.is_empty());
    }

    /// Larger list (> 127 entities) forces the varint length prefix to spill
    /// into a second byte. Ensures the length encoding actually handles the
    /// multi-byte varint case rather than accidentally working only for small
    /// counts.
    #[test]
    fn chunk_assembled_frame_with_multi_byte_varint_length() {
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
            };
            chunks.push(encode_entity(&e));
            entities.push(e);
        }
        let chunk_refs: Vec<&[u8]> = chunks.iter().map(|c| c.as_slice()).collect();

        let via_chunks = encode_server_delta_from_chunks(&header, &chunk_refs, &[]).unwrap();

        let via_full = encode_server(&ServerFrame::Delta(DeltaPayload {
            source_cluster_id: header.source_cluster_id,
            seq: header.seq,
            tick: header.tick,
            timestamp: header.timestamp,
            updated: entities.clone(),
            removed: Vec::new(),
        }))
        .unwrap();

        assert_eq!(via_chunks, via_full);

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.updated.len(), 200);
        assert_eq!(payload.updated[150], entities[150]);
    }

    /// A subscriber selecting a subset of entities — the AOI use case. The
    /// subset-of-chunks frame must be a valid, decodable frame containing
    /// only the selected entities.
    #[test]
    fn chunk_assembled_frame_supports_subset_selection() {
        let header = sample_header();
        let e1 = EntityState {
            entity_id: Uuid::from_u128(1),
            cluster_id: Uuid::nil(),
            position: q3(1.0, 0.0, 0.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
        };
        let e2 = EntityState {
            entity_id: Uuid::from_u128(2),
            cluster_id: Uuid::nil(),
            position: q3(2.0, 0.0, 0.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
        };
        let e3 = EntityState {
            entity_id: Uuid::from_u128(3),
            cluster_id: Uuid::nil(),
            position: q3(3.0, 0.0, 0.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
        };
        let c1 = encode_entity(&e1);
        let c2 = encode_entity(&e2);
        let c3 = encode_entity(&e3);

        // Producer pre-encodes all three entities once. Subscriber takes only
        // the first and third — the AOI filter result.
        let via_chunks =
            encode_server_delta_from_chunks(&header, &[c1.as_slice(), c3.as_slice()], &[]).unwrap();

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.updated.len(), 2);
        assert_eq!(payload.updated[0], e1);
        assert_eq!(payload.updated[1], e3);

        // Sanity: the other chunk reference still produces its own independent
        // encoding — per-entity chunks are not consumed by assembly.
        assert!(!c2.is_empty(), "unused chunks remain usable");
    }

    /// Chunks carrying non-empty `user_data` (JSON bytes today) must pass
    /// through correctly. Guards against accidental user_data mangling in the
    /// splice path.
    #[test]
    fn chunk_assembled_frame_preserves_user_data_bytes() {
        let header = sample_header();
        let e = EntityState {
            entity_id: Uuid::from_u128(42),
            cluster_id: Uuid::from_u128(99),
            position: q3(1.0, 2.0, 3.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: br#"{"hp":99,"status":"poisoned"}"#.to_vec(),
        };
        let chunk = encode_entity(&e);

        let via_chunks =
            encode_server_delta_from_chunks(&header, &[chunk.as_slice()], &[]).unwrap();

        let decoded = decode_server(&via_chunks).unwrap();
        let ServerFrame::Delta(payload) = decoded;
        assert_eq!(payload.updated.len(), 1);
        assert_eq!(payload.updated[0].user_data, e.user_data);
    }

    // ── Vec3Q: quantization properties at the wire boundary ──

    #[test]
    fn vec3q_roundtrip_within_quantization_step() {
        // Sub-unit components round to the nearest i16 step. Loss is
        // bounded by 0.5 unit per component.
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
    fn vec3q_is_smaller_on_wire_than_vec3() {
        // Postcard varint-encodes integers (zigzag for signed). Small i16
        // components pack into 1 byte each; near-i16-max into 3 bytes each.
        // Either way Vec3Q is meaningfully smaller than Vec3 (24 bytes
        // fixed-width). The exact saving depends on the value distribution
        // — we just guard that the size relationship holds.
        let near_origin = Vec3Q::new(1, 2, 3);
        let near_world_max = Vec3Q::new(5000, 5000, 5000);
        let f64_v = Vec3::new(1.0, 2.0, 3.0);

        let q_origin_bytes = postcard::to_allocvec(&near_origin).unwrap();
        let q_max_bytes = postcard::to_allocvec(&near_world_max).unwrap();
        let f64_bytes = postcard::to_allocvec(&f64_v).unwrap();

        assert!(q_origin_bytes.len() < f64_bytes.len());
        assert!(q_max_bytes.len() < f64_bytes.len());
        assert_eq!(f64_bytes.len(), 24);
    }

    #[test]
    fn decode_rejects_truncated_bytes() {
        let frame = ClientFrame::PlayerState(PlayerStatePayload {
            entity_id: Uuid::from_u128(1),
            position: q3(1.0, 2.0, 3.0),
            velocity: q3(0.0, 0.0, 0.0),
            user_data: Vec::new(),
        });
        let bytes = encode_client(&frame).unwrap();
        let truncated = &bytes[..bytes.len() - 1];
        let result = decode_client(truncated);
        assert!(
            result.is_err(),
            "truncated postcard bytes should fail to decode"
        );
    }
}
