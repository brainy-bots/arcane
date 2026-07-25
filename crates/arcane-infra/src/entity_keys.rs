//! Per-entity state records with owner-enforced writes (issue #331 design,
//! founder-settled 2026-07-25).
//!
//! Ownership is a FIELD ON THE ENTITY RECORD, not a separate table and not a
//! node-side state machine. One key per entity:
//!
//!   arcane:entity:<entity_id>  (hash: owner, tick, doc)
//!
//! `doc` is the full serialized EntityRecord — written WHOLESALE every
//! publish. There is no snapshot+delta protocol: a keyed store IS a snapshot
//! store, the last write is always the complete current state, and a
//! migrating entity's new owner bootstraps from one read (or the inbox
//! frame's force-included copy). The store cannot represent a partial entity.
//!
//! Single-writer is enforced BY REDIS (single-threaded, atomic Lua): a write
//! carries the writer's cluster id and is REJECTED if the record's `owner`
//! names someone else. The unsynchronized-delivery race — the old owner keeps
//! simulating for a frame or two after the manager flips ownership — becomes
//! harmless: the late writer bounces off the store. No hot-path work is added
//! to the node; the same async publisher-thread pattern applies, with
//! pipelined EVALSHA batches.
//!
//! Ownership transfer is a HANDOFF PERFORMED BY THE OWNER — the only party
//! that can write. When the manager's statement releases an entity from node
//! A to node B, A's final act for that entity is ONE atomic write: its last
//! simulated state PLUS `owner = B`. Consequences:
//!
//! - The old owner's final frame is preserved (it ships WITH the transfer),
//!   so the entity's authoritative history has no hole at the seam.
//! - B may believe it owns the entity before the handoff lands (the manager
//!   told it so) and will optimistically write — those writes FAIL at the
//!   gate, which is correct and free: the check runs in Redis, never on the
//!   node's hot path. B's writes start succeeding the instant A's handoff
//!   sets the owner field. No claim, no negotiation, no node-side state.
//! - Crash safety comes from the TTL below, not from a claim: if A dies
//!   mid-handoff the record expires and B's next gated write takes it
//!   (a fresh/absent record has no owner to reject).
//!
//! Departure cleanup is TTL-based: every write refreshes a short TTL, so an
//! entity that stops being written (left the game, idle-despawned) expires
//! from the store without bookkeeping. Migrating entities never expire — the
//! new owner's writes refresh the same key.
//!
//! A per-cluster heartbeat key (arcane:clustertick:<cluster_id>) carries the
//! node's tick for staleness detection, replacing the blob doc's tick field.

use arcane_affinity::feature_map::EntityRecord;
use std::sync::mpsc;
use std::thread;
use uuid::Uuid;

/// Redis key for one entity's state record.
pub fn entity_key(entity_id: Uuid) -> String {
    format!("arcane:entity:{}", entity_id.hyphenated())
}

/// Redis key for a cluster's tick heartbeat (staleness detection).
pub fn cluster_tick_key(cluster_id: Uuid) -> String {
    format!("arcane:clustertick:{}", cluster_id.hyphenated())
}

/// Seconds a record lives without a refresh. Nodes publish owned entities at
/// 20Hz, so 5s of silence means the entity truly left (or its owner died —
/// in which case the manager has long since reassigned it and the new
/// owner's writes keep the key alive).
pub const ENTITY_TTL_SECS: u64 = 5;

/// Owner-gated write: full-record upsert REJECTED unless the standing
/// record's owner is the writer (or the record is fresh/expired).
/// KEYS[1] = entity key; ARGV[1] = writer cluster id, ARGV[2] = doc JSON,
/// ARGV[3] = node tick, ARGV[4] = TTL secs. Returns 1 = written, 0 = rejected.
pub const WRITE_SCRIPT: &str = r#"
local cur = redis.call('HGET', KEYS[1], 'owner')
if cur and cur ~= ARGV[1] then return 0 end
redis.call('HSET', KEYS[1], 'owner', ARGV[1], 'doc', ARGV[2], 'tick', ARGV[3])
redis.call('EXPIRE', KEYS[1], ARGV[4])
return 1
"#;

/// HANDOFF: the owner's final write for an entity it is releasing — last
/// simulated state AND the ownership transfer, atomically. Still
/// OWNER-GATED: only the current owner may hand off (a node that is not the
/// owner cannot move ownership, by construction).
/// KEYS[1] = entity key; ARGV[1] = writer (current owner), ARGV[2] = doc
/// JSON, ARGV[3] = tick, ARGV[4] = TTL secs, ARGV[5] = NEW owner.
/// Returns 1 = handed off, 0 = rejected (not the owner).
pub const HANDOFF_SCRIPT: &str = r#"
local cur = redis.call('HGET', KEYS[1], 'owner')
if cur and cur ~= ARGV[1] then return 0 end
redis.call('HSET', KEYS[1], 'owner', ARGV[5], 'doc', ARGV[2], 'tick', ARGV[3])
redis.call('EXPIRE', KEYS[1], ARGV[4])
return 1
"#;

/// One write operation for the publisher thread.
pub enum EntityWriteOp {
    /// Owner-gated full-record write (the per-publish path).
    Write {
        entity_id: Uuid,
        doc_json: String,
        tick: u64,
    },
    /// Handoff: final state + ownership transfer, issued by the CURRENT
    /// owner when the manager's statement releases the entity.
    Handoff {
        entity_id: Uuid,
        doc_json: String,
        tick: u64,
        new_owner: Uuid,
    },
    /// Cluster heartbeat (once per publish batch).
    Heartbeat { tick: u64 },
}

/// Async per-entity state publisher: same non-blocking pattern as the blob
/// `StatePublisher` — the node enqueues, a dedicated thread owns the Redis
/// connection and drains the queue with pipelined script calls.
pub struct EntityKeyPublisher {
    tx: mpsc::Sender<Vec<EntityWriteOp>>,
}

impl EntityKeyPublisher {
    pub fn new(redis_url: &str, cluster_id: Uuid) -> Result<Self, String> {
        let client =
            redis::Client::open(redis_url).map_err(|e| format!("Redis open failed: {e}"))?;
        let (tx, rx) = mpsc::channel::<Vec<EntityWriteOp>>();
        let me = cluster_id.hyphenated().to_string();

        thread::spawn(move || {
            let write_script = redis::Script::new(WRITE_SCRIPT);
            let handoff_script = redis::Script::new(HANDOFF_SCRIPT);
            let mut conn: Option<redis::Connection> = client.get_connection().ok();
            let mut rejected_total: u64 = 0;
            while let Ok(batch) = rx.recv() {
                if conn.is_none() {
                    conn = client.get_connection().ok();
                }
                let Some(c) = conn.as_mut() else { continue };
                let mut failed = false;
                for op in &batch {
                    let result: Result<i64, redis::RedisError> = match op {
                        EntityWriteOp::Write {
                            entity_id,
                            doc_json,
                            tick,
                        } => write_script
                            .key(entity_key(*entity_id))
                            .arg(&me)
                            .arg(doc_json)
                            .arg(*tick)
                            .arg(ENTITY_TTL_SECS)
                            .invoke(c),
                        EntityWriteOp::Handoff {
                            entity_id,
                            doc_json,
                            tick,
                            new_owner,
                        } => handoff_script
                            .key(entity_key(*entity_id))
                            .arg(&me)
                            .arg(doc_json)
                            .arg(*tick)
                            .arg(ENTITY_TTL_SECS)
                            .arg(new_owner.hyphenated().to_string())
                            .invoke(c),
                        EntityWriteOp::Heartbeat { tick } => redis::cmd("SET")
                            .arg(cluster_tick_key(Uuid::parse_str(&me).unwrap_or_default()))
                            .arg(*tick)
                            .query(c)
                            .map(|(): ()| 1i64),
                    };
                    match result {
                        Ok(0) => {
                            // Owner-gated rejection: WORKING AS DESIGNED (we
                            // are a stale writer post-flip). Log sparsely.
                            rejected_total += 1;
                            if rejected_total.is_multiple_of(100) || rejected_total == 1 {
                                eprintln!(
                                    "entity-keys: {rejected_total} stale writes rejected by owner gate (single-writer invariant holding)"
                                );
                            }
                        }
                        Ok(_) => {}
                        Err(_) => {
                            failed = true;
                            break;
                        }
                    }
                }
                if failed {
                    conn = None; // rebuild next batch
                }
            }
        });

        Ok(Self { tx })
    }

    /// Enqueue a batch (non-blocking).
    pub fn publish(&self, ops: Vec<EntityWriteOp>) -> Result<(), String> {
        self.tx
            .send(ops)
            .map_err(|_| "entity-key publisher thread dead".to_string())
    }
}

/// Serialize an EntityRecord as the record doc.
pub fn encode_record(record: &EntityRecord) -> Result<String, String> {
    serde_json::to_string(record).map_err(|e| format!("encode failed: {e}"))
}

/// Reader: fetch ALL entity records (SCAN + pipelined HGET). Used by the
/// manager and router state source in entity-keys mode. Returns records plus
/// per-cluster heartbeat ticks.
pub fn fetch_all_entities(
    conn: &mut redis::Connection,
    known_clusters: &[Uuid],
) -> (Vec<EntityRecord>, Vec<(Uuid, u64)>) {
    let mut records = Vec::new();
    let mut cursor: u64 = 0;
    while let Ok((next, keys)) = redis::cmd("SCAN")
        .arg(cursor)
        .arg("MATCH")
        .arg("arcane:entity:*")
        .arg("COUNT")
        .arg(500)
        .query::<(u64, Vec<String>)>(conn)
    {
        if !keys.is_empty() {
            let mut pipe = redis::pipe();
            for k in &keys {
                pipe.cmd("HGET").arg(k).arg("doc");
            }
            match pipe.query::<Vec<Option<String>>>(conn) {
                Ok(docs) => {
                    for doc in docs.into_iter().flatten() {
                        match serde_json::from_str::<EntityRecord>(&doc) {
                            Ok(rec) => records.push(rec),
                            Err(e) => {
                                // NEVER swallow this: a decode mismatch here
                                // empties the manager's whole world view and
                                // looks like “all entities on one cluster,
                                // no migrations” (live-hit 2026-07-25 when
                                // records serialized without optional fields
                                // failed to deserialize).
                                use std::sync::atomic::{AtomicU64, Ordering};
                                static DECODE_FAILS: AtomicU64 = AtomicU64::new(0);
                                let n = DECODE_FAILS.fetch_add(1, Ordering::Relaxed);
                                if n.is_multiple_of(200) {
                                    eprintln!(
                                        "entity-keys reader: record decode FAILED ({e}); doc={}",
                                        &doc[..doc.len().min(200)]
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("entity-keys reader: HGET pipeline failed: {e}");
                }
            }
        }
        cursor = next;
        if cursor == 0 {
            break;
        }
    }

    let mut ticks = Vec::new();
    for &c in known_clusters {
        if let Ok(Some(t)) = redis::cmd("GET")
            .arg(cluster_tick_key(c))
            .query::<Option<u64>>(conn)
        {
            ticks.push((c, t));
        }
    }
    (records, ticks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_formats() {
        let id = Uuid::nil();
        assert_eq!(
            entity_key(id),
            "arcane:entity:00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(
            cluster_tick_key(id),
            "arcane:clustertick:00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn write_script_owner_gates() {
        // Static sanity: the write script checks owner before writing and
        // returns 0 on mismatch — the single-writer invariant lives HERE.
        assert!(WRITE_SCRIPT.contains("if cur and cur ~= ARGV[1] then return 0 end"));
        assert!(WRITE_SCRIPT.contains("EXPIRE"));
        // The handoff script is ALSO owner-gated: only the current owner may
        // transfer ownership (a non-owner cannot write, therefore cannot move
        // the field). It sets the NEW owner from ARGV[5].
        assert!(HANDOFF_SCRIPT.contains("if cur and cur ~= ARGV[1] then return 0 end"));
        assert!(HANDOFF_SCRIPT.contains("'owner', ARGV[5]"));
        assert!(HANDOFF_SCRIPT.contains("EXPIRE"));
    }

    #[test]
    fn record_roundtrip() {
        let rec = EntityRecord {
            entity_id: Uuid::from_u128(7),
            cluster_id: Uuid::from_u128(1),
            position: arcane_core::types::Vec2::new(1.0, 2.0),
            velocity: arcane_core::types::Vec2::new(0.5, -0.5),
            features: arcane_affinity::feature_map::FeatureMap::new(),
            user_data: serde_json::Value::Null,
        };
        let json = encode_record(&rec).unwrap();
        let back: EntityRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.entity_id, rec.entity_id);
        assert_eq!(back.cluster_id, rec.cluster_id);
    }
}
