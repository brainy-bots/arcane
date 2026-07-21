//! D1 — the forwarding invariant (epic #287, Session Relay L0).
//!
//! **Invariant: inputs that arrive at a non-owner node for an entity are
//! forwarded to the entity's current owner. The non-owner NEVER applies them
//! locally.**
//!
//! Why this exists: a player entity has TWO writers — the simulation and a
//! client connection anchored to one node. Migration moves only the first.
//! Without forwarding, a connected player's inputs keep arriving at the OLD
//! owner after a flip, which either mis-applies them (split-brain: two live
//! copies, observers permanently disagree — demonstrated on the wire by
//! `examples/migration_observer.rs`) or silently drops them (the entity
//! freezes on its new owner). Forwarding closes the gap: the old owner
//! relays the input to the current owner over Redis, and the entity keeps
//! moving exactly as if the client were connected to the new owner.
//!
//! This is a *correctness* property of single-writer ownership, not an
//! optimization — and it is the prerequisite that makes every future ingress
//! topology race-free (a RECONNECT redirect or a relay upstream switch can be
//! lazy because stray inputs during the window still reach the owner).
//!
//! Transport follows the [`crate::physics_events_channel`] pattern: JSON over
//! Redis pub/sub, non-blocking publisher thread, thread-spawned subscriber,
//! mpsc forwarding. Topic: `arcane:fwd_inputs:<cluster_uuid>` (point-to-point).
//!
//! **Loop safety (structural):** forwarded inputs are delivered on a channel
//! that is drained WITHOUT the forwarding check — a forwarded input is either
//! applied (receiver owns the entity) or dropped and counted (ownership moved
//! again mid-flight; the client's next input, arriving at 10-20Hz, forwards
//! to the right place). At most one forward hop per input, no ping-pong.

use std::sync::mpsc::Sender;
use std::thread;

use arcane_core::cluster_simulation::GameAction;
use arcane_core::replication_channel::EntityStateEntry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const FWD_INPUTS_TOPIC_PREFIX: &str = "arcane:fwd_inputs";

/// One forwarded PLAYER_STATE update. `EntityStateEntry.client_seq` is
/// `#[serde(skip)]` (out-of-band, never meant for the replication paths),
/// so the relay carries it EXPLICITLY — otherwise forwarded inputs would
/// arrive at the owner with seq 0 and break round-trip latency measurement
/// for exactly the players that migrated.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForwardedUpdate {
    pub entry: EntityStateEntry,
    pub client_seq: u64,
}

impl ForwardedUpdate {
    /// Capture the seq from the entry before serde strips it.
    pub fn new(entry: EntityStateEntry) -> Self {
        let client_seq = entry.client_seq;
        Self { entry, client_seq }
    }

    /// Restore the seq onto the entry after deserialization.
    pub fn into_entry(mut self) -> EntityStateEntry {
        self.entry.client_seq = self.client_seq;
        self.entry
    }
}

/// One batch of client inputs relayed from a non-owner node to the owner.
/// Batched per drain pass (one publish per target cluster per tick, not per
/// input) to keep the Redis hot path proportional to clusters, not clients.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForwardedInputBatch {
    /// The node that received the inputs from the client and relayed them.
    pub source_cluster_id: Uuid,
    /// PLAYER_STATE entries for entities the source does not own.
    pub updates: Vec<ForwardedUpdate>,
    /// GAME_ACTION entries for entities the source does not own.
    pub actions: Vec<GameAction>,
}

impl ForwardedInputBatch {
    pub fn new(source_cluster_id: Uuid) -> Self {
        Self {
            source_cluster_id,
            updates: Vec::new(),
            actions: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.updates.is_empty() && self.actions.is_empty()
    }
}

/// Message queued for the publisher thread.
struct PublishMessage {
    target_cluster_id: Uuid,
    batch: ForwardedInputBatch,
}

/// Publishes forwarded input batches to owner clusters via Redis.
/// Non-blocking: batches are enqueued on a producer thread via mpsc, which
/// owns the Redis connection and drains the queue (same lifecycle as
/// [`crate::physics_events_channel::PhysicsEventsPublisher`]).
pub struct ForwardedInputsPublisher {
    tx: std::sync::mpsc::Sender<PublishMessage>,
}

impl ForwardedInputsPublisher {
    pub fn new(redis_url: &str) -> Result<Self, String> {
        let client =
            redis::Client::open(redis_url).map_err(|e| format!("Redis open failed: {}", e))?;
        let (tx, rx) = std::sync::mpsc::channel::<PublishMessage>();

        thread::spawn(move || {
            // Lazily (re)connect; never exit on connection failure so `forward()`
            // can always enqueue.
            let mut conn: Option<redis::Connection> = client.get_connection().ok();
            while let Ok(msg) = rx.recv() {
                if msg.batch.is_empty() {
                    continue;
                }
                if conn.is_none() {
                    conn = client.get_connection().ok();
                }
                let Some(c) = conn.as_mut() else {
                    continue;
                };
                if let Ok(payload) = serde_json::to_string(&msg.batch) {
                    let topic = format!("{}:{}", FWD_INPUTS_TOPIC_PREFIX, msg.target_cluster_id);
                    let res: Result<i64, redis::RedisError> =
                        redis::cmd("PUBLISH").arg(&topic).arg(&payload).query(c);
                    if res.is_err() {
                        conn = None;
                    }
                }
            }
        });

        Ok(Self { tx })
    }

    /// Enqueue a batch for non-blocking relay to the owner cluster.
    pub fn forward(
        &self,
        target_cluster_id: Uuid,
        batch: ForwardedInputBatch,
    ) -> Result<(), String> {
        if batch.is_empty() {
            return Ok(());
        }
        self.tx
            .send(PublishMessage {
                target_cluster_id,
                batch,
            })
            .map_err(|_| "publisher thread dead".to_string())
    }
}

/// Spawn a background thread subscribing to this cluster's forwarded-inputs
/// topic. Batches are delivered raw; the ownership check happens at drain time
/// on the driver thread (single decision point, and structurally outside the
/// forwarding path — see the loop-safety note in the module docs).
pub fn spawn_forwarded_inputs_subscriber(
    redis_url: String,
    self_cluster_id: Uuid,
    tx: Sender<ForwardedInputBatch>,
) {
    thread::spawn(move || {
        let client = match redis::Client::open(redis_url.as_str()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("forwarded inputs subscriber: Redis open failed: {}", e);
                return;
            }
        };
        let mut conn = match client.get_connection() {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "forwarded inputs subscriber: Redis connection failed: {}",
                    e
                );
                return;
            }
        };
        let mut pubsub = conn.as_pubsub();
        let topic = format!("{}:{}", FWD_INPUTS_TOPIC_PREFIX, self_cluster_id);
        if pubsub.subscribe(&topic).is_err() {
            eprintln!("forwarded inputs subscriber: subscribe {} failed", topic);
            return;
        }
        eprintln!("subscribed to forwarded inputs topic {}", topic);
        loop {
            match pubsub.get_message() {
                Ok(msg) => {
                    let payload: String = match msg.get_payload() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if let Ok(batch) = serde_json::from_str::<ForwardedInputBatch>(&payload) {
                        if tx.send(batch).is_err() {
                            break; // node core dropped its receiver
                        }
                    }
                }
                Err(e) => {
                    eprintln!("forwarded inputs subscriber: get_message error: {}", e);
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_core::types::Vec3;

    fn entry(id: Uuid) -> EntityStateEntry {
        EntityStateEntry::new(
            id,
            Uuid::nil(),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(0.1, 0.0, 0.0),
        )
    }

    #[test]
    fn forwarded_update_preserves_client_seq_through_json() {
        // EntityStateEntry.client_seq is #[serde(skip)]; the relay must
        // carry it out-of-band or latency measurement silently breaks for
        // migrated players. This is the regression test for that bug.
        let mut e = entry(Uuid::new_v4());
        e.client_seq = 4242;
        let fwd = ForwardedUpdate::new(e);
        let json = serde_json::to_string(&fwd).unwrap();
        let back: ForwardedUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.into_entry().client_seq, 4242);
    }

    #[test]
    fn batch_roundtrips_through_json() {
        let src = Uuid::new_v4();
        let eid = Uuid::new_v4();
        let mut batch = ForwardedInputBatch::new(src);
        batch.updates.push(ForwardedUpdate::new(entry(eid)));
        batch.actions.push(GameAction {
            entity_id: eid,
            action_type: "cast_spell".into(),
            payload: serde_json::json!({"spell": 3}),
        });

        let json = serde_json::to_string(&batch).unwrap();
        let back: ForwardedInputBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source_cluster_id, src);
        assert_eq!(back.updates.len(), 1);
        assert_eq!(back.updates[0].entry.entity_id, eid);
        assert_eq!(back.actions.len(), 1);
        assert_eq!(back.actions[0].action_type, "cast_spell");
    }

    #[test]
    fn empty_batch_is_empty() {
        let batch = ForwardedInputBatch::new(Uuid::new_v4());
        assert!(batch.is_empty());
    }

    #[test]
    fn publisher_enqueues_without_blocking_even_without_redis() {
        // Same non-blocking guarantee as the physics/inbox publishers: creating
        // the publisher against a dead Redis must not block or panic, and
        // forward() must return immediately (enqueue semantics).
        let publisher = match ForwardedInputsPublisher::new("redis://127.0.0.1:16399") {
            Ok(p) => p,
            Err(_) => return, // Client::open rejected the URL — acceptable
        };
        let mut batch = ForwardedInputBatch::new(Uuid::new_v4());
        batch
            .updates
            .push(ForwardedUpdate::new(entry(Uuid::new_v4())));
        let started = std::time::Instant::now();
        publisher.forward(Uuid::new_v4(), batch).unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "forward() must be non-blocking"
        );
    }
}
