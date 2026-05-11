//! SpacetimeDB persistence adapter for throttled state snapshots.
//!
//! Responsibilities:
//! - derive persist cadence and endpoint config from environment
//! - encode `EntityStateEntry` batches into SpacetimeDB reducer payload shape
//! - send chunked HTTP requests and log success/failure totals
//!
//! **Four buckets:** this path mirrors **bucket 1** (pose) *and* **bucket 2** (`user_data`) into
//! **bucket 4** (durable tables) at a throttled cadence — not a substitute for hot Redis
//! replication between clusters. The target SpacetimeDB module's `Entity` table must include
//! columns matching the fields this encoder emits (`entity_id`, `x`, `y`, `z`, `user_data`).
//!
//! **Progressive-API note (see `docs/architecture/progressive-api.md`):** this is the level-1
//! auto-persist path. Level-0 users get positions for free; level-1 users put any additional
//! per-entity state in `EntityStateEntry::user_data` and it rides along in the same snapshot.
//! Level-2+ (explicit flush trigger, custom reducer, typed schemas) is deferred until a real
//! game-driven use case demands it.
//!
//! This module does not own simulation timing; `cluster_runner` decides when to call it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use arcane_core::replication_channel::EntityStateEntry;

#[derive(serde::Serialize)]
struct SpacetimeUuid {
    __uuid__: u128,
}

#[derive(serde::Serialize)]
struct SpacetimeEntityRow {
    entity_id: SpacetimeUuid,
    x: f64,
    y: f64,
    z: f64,
    /// Serialized `EntityStateEntry::user_data` (JSON). Empty string when the entry has no
    /// user_data. Keeping it as a string (not a structured type) lets the SpacetimeDB module
    /// column be a simple `String` that any game can stuff its own JSON into without this
    /// library knowing the schema.
    user_data: String,
}

fn encode_user_data(value: &serde_json::Value) -> String {
    if value.is_null() {
        String::new()
    } else {
        serde_json::to_string(value).unwrap_or_default()
    }
}

fn encode_spacetimedb_entities_body(
    chunk: &[EntityStateEntry],
) -> Result<String, serde_json::Error> {
    // Snapshot carries pose (bucket 1) plus user_data (bucket 2). The SpacetimeDB module's
    // Entity table is expected to have a `user_data: String` column; games that don't use
    // bucket 2 get an empty string and pay no meaningful cost.
    let entities: Vec<SpacetimeEntityRow> = chunk
        .iter()
        .map(|e| SpacetimeEntityRow {
            entity_id: SpacetimeUuid {
                __uuid__: u128::from_be_bytes(*e.entity_id.as_bytes()),
            },
            x: e.position.x,
            y: e.position.y,
            z: e.position.z,
            user_data: encode_user_data(&e.user_data),
        })
        .collect();
    serde_json::to_string(&vec![entities])
}

fn should_persist_tick(tick: u64, interval_ticks: u64, entries_len: usize) -> bool {
    tick.is_multiple_of(interval_ticks) && entries_len > 0
}

pub struct SpacetimeDbPersist {
    sender: mpsc::SyncSender<Vec<EntityStateEntry>>,
    interval_ticks: u64,
}

impl SpacetimeDbPersist {
    pub fn from_env() -> Option<Self> {
        let uri =
            std::env::var("SPACETIMEDB_URI").unwrap_or_else(|_| "http://127.0.0.1:3000".into());
        let db = std::env::var("SPACETIMEDB_DATABASE").unwrap_or_else(|_| "arcane".into());
        let hz: u64 = std::env::var("SPACETIMEDB_PERSIST_HZ")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let enabled = std::env::var("SPACETIMEDB_PERSIST")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !enabled {
            return None;
        }
        // Persist cadence is computed from the resolved cluster tick rate so the
        // SpacetimeDB snapshot rate stays at the requested Hz regardless of the
        // simulation tick rate. e.g. cluster at 30 Hz + persist at 1 Hz =
        // every 30 ticks.
        let tick_rate_hz = crate::tick_rate::tick_rate_hz();
        let interval_ticks = (tick_rate_hz / hz.max(1)).max(1);
        let max_batch_size: usize = std::env::var("SPACETIMEDB_PERSIST_BATCH_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let url = format!(
            "{}/v1/database/{}/call/set_entities",
            uri.trim_end_matches('/'),
            db
        );
        eprintln!(
            "SpacetimeDB persist: {} (every {} ticks = {} Hz, batch_size={})",
            url,
            interval_ticks,
            hz,
            if max_batch_size == 0 {
                "unlimited".to_string()
            } else {
                max_batch_size.to_string()
            }
        );

        let (tx, rx) = mpsc::sync_channel::<Vec<EntityStateEntry>>(2);
        let persist_count = std::sync::Arc::new(AtomicU64::new(0));
        let persist_count_bg = persist_count.clone();

        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("reqwest client");

            while let Ok(entries) = rx.recv() {
                let count = persist_count_bg.fetch_add(1, Ordering::Relaxed);
                let log_this_cycle = count % 10 == 0;
                Self::persist_entries(&client, &url, &entries, max_batch_size, log_this_cycle);
            }
        });

        Some(Self {
            sender: tx,
            interval_ticks,
        })
    }

    fn persist_entries(
        client: &reqwest::blocking::Client,
        url: &str,
        entries: &[EntityStateEntry],
        max_batch_size: usize,
        log_stats: bool,
    ) {
        if entries.is_empty() {
            return;
        }

        let chunk_size = if max_batch_size > 0 {
            max_batch_size
        } else {
            entries.len()
        };

        let t0 = Instant::now();
        let mut ok_count = 0usize;
        let mut err_count = 0usize;

        for chunk in entries.chunks(chunk_size) {
            let body = match encode_spacetimedb_entities_body(chunk) {
                Ok(s) => s,
                Err(e) => {
                    err_count += chunk.len();
                    eprintln!("SpacetimeDB persist serialization error: {}", e);
                    continue;
                }
            };
            match client
                .post(url)
                .header("Content-Type", "application/json")
                .body(body)
                .send()
            {
                Ok(resp) if resp.status().is_success() => ok_count += chunk.len(),
                Ok(resp) => {
                    err_count += chunk.len();
                    eprintln!("SpacetimeDB persist error: HTTP {}", resp.status());
                }
                Err(e) => {
                    err_count += chunk.len();
                    eprintln!("SpacetimeDB persist error: {}", e);
                }
            }
        }

        if log_stats {
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            let chunks = entries.len().div_ceil(chunk_size);
            eprintln!(
                "SpacetimeDB persist: {} entities ({} chunk(s), max {} each) in {:.1}ms",
                ok_count + err_count,
                chunks,
                chunk_size,
                ms
            );
        }
    }

    pub fn maybe_persist(&self, tick: u64, entries: &[EntityStateEntry]) {
        if !should_persist_tick(tick, self.interval_ticks, entries.len()) {
            return;
        }

        match self.sender.try_send(entries.to_vec()) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                eprintln!("SpacetimeDB persist: channel full, dropping snapshot");
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                eprintln!("SpacetimeDB persist: background thread exited");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_core::replication_channel::EntityStateEntry;
    use arcane_core::Vec3;
    use uuid::Uuid;

    fn mk_entry(id: Uuid, x: f64, y: f64, z: f64) -> EntityStateEntry {
        EntityStateEntry::new(
            id,
            Uuid::nil(),
            Vec3::new(x, y, z),
            Vec3::new(9.0, 9.0, 9.0),
        )
    }

    #[test]
    fn encode_spacetimedb_entities_body_uses_expected_shape() {
        let id = Uuid::from_u128(1);
        let body = encode_spacetimedb_entities_body(&[mk_entry(id, 1.5, 2.5, 3.5)]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();

        // API payload shape is [[{entity...}]].
        assert!(v.is_array());
        assert_eq!(v.as_array().unwrap().len(), 1);
        let batch = &v.as_array().unwrap()[0];
        assert!(batch.is_array());
        assert_eq!(batch.as_array().unwrap().len(), 1);

        let row = &batch.as_array().unwrap()[0];
        assert_eq!(row["entity_id"]["__uuid__"].as_u64().unwrap(), 1);
        assert_eq!(row["x"].as_f64().unwrap(), 1.5);
        assert_eq!(row["y"].as_f64().unwrap(), 2.5);
        assert_eq!(row["z"].as_f64().unwrap(), 3.5);
    }

    #[test]
    fn encode_spacetimedb_entities_body_emits_all_entities() {
        let e1 = mk_entry(Uuid::from_u128(10), 1.0, 1.0, 1.0);
        let e2 = mk_entry(Uuid::from_u128(11), 2.0, 2.0, 2.0);
        let body = encode_spacetimedb_entities_body(&[e1, e2]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v[0].as_array().unwrap().len(), 2);
    }

    #[test]
    fn encode_spacetimedb_entities_body_carries_user_data() {
        // Null user_data → empty string (bucket 2 not used by this game).
        let mut null_entry = mk_entry(Uuid::from_u128(20), 0.0, 0.0, 0.0);
        null_entry.user_data = serde_json::Value::Null;

        // Populated user_data → JSON-encoded string column.
        let mut rich_entry = mk_entry(Uuid::from_u128(21), 0.0, 0.0, 0.0);
        rich_entry.user_data = serde_json::json!({ "hp": 73, "inv": ["sword"] });

        let body = encode_spacetimedb_entities_body(&[null_entry, rich_entry]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let rows = v[0].as_array().unwrap();
        assert_eq!(rows[0]["user_data"].as_str().unwrap(), "");
        let parsed: serde_json::Value =
            serde_json::from_str(rows[1]["user_data"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["hp"], 73);
        assert_eq!(parsed["inv"][0], "sword");
    }

    #[test]
    fn should_persist_tick_obeys_cadence_and_non_empty_entries() {
        assert!(should_persist_tick(0, 20, 1));
        assert!(should_persist_tick(20, 20, 5));
        assert!(!should_persist_tick(1, 20, 5));
        assert!(!should_persist_tick(19, 20, 5));
        assert!(!should_persist_tick(20, 20, 0));
    }

    #[test]
    fn maybe_persist_delivers_entries_through_channel() {
        let (tx, rx) = mpsc::sync_channel::<Vec<EntityStateEntry>>(2);
        let persist = SpacetimeDbPersist {
            sender: tx,
            interval_ticks: 1,
        };

        let entry = mk_entry(Uuid::from_u128(100), 1.0, 2.0, 3.0);
        persist.maybe_persist(0, &[entry.clone()]);

        let received = rx.try_recv().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].entity_id, entry.entity_id);
        assert_eq!(received[0].position.x, 1.0);
    }

    #[test]
    fn maybe_persist_drops_on_full_channel() {
        let (tx, _rx) = mpsc::sync_channel::<Vec<EntityStateEntry>>(1);
        let persist = SpacetimeDbPersist {
            sender: tx,
            interval_ticks: 1,
        };

        let entry = mk_entry(Uuid::from_u128(101), 0.0, 0.0, 0.0);
        // Fill the channel
        persist.maybe_persist(0, &[entry.clone()]);
        // Second call should drop without blocking (channel full)
        let t0 = Instant::now();
        persist.maybe_persist(1, &[entry]);
        let elapsed = t0.elapsed().as_millis();
        assert!(elapsed < 10, "maybe_persist blocked on full channel: {}ms", elapsed);
    }

    #[test]
    fn maybe_persist_is_nonblocking() {
        let (tx, _rx) = std::sync::mpsc::sync_channel::<Vec<EntityStateEntry>>(1);

        let persist = SpacetimeDbPersist {
            sender: tx,
            interval_ticks: 1,
        };

        let entry = mk_entry(Uuid::from_u128(102), 0.0, 0.0, 0.0);
        let entries = vec![entry];

        let t0 = Instant::now();
        persist.maybe_persist(0, &entries);
        let elapsed = t0.elapsed().as_millis();

        assert!(
            elapsed < 10,
            "maybe_persist should complete in < 10ms, took {}ms",
            elapsed
        );
    }
}
