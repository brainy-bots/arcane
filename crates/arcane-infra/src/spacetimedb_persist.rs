//! SpacetimeDB persistence adapter for throttled state snapshots and neighbor bootstrap.
//!
//! Responsibilities:
//! - derive persist cadence and endpoint config from environment
//! - encode `EntityStateEntry` batches into SpacetimeDB reducer payload shape
//! - send chunked HTTP requests and log success/failure totals
//! - read entity snapshots for neighbor bootstrap (gap recovery)
//!
//! **Four buckets:** this path mirrors **bucket 1** (pose) *and* **bucket 2** (`user_data`) into
//! **bucket 4** (durable tables) at a throttled cadence — not a substitute for hot Redis
//! replication between clusters. The target SpacetimeDB module's `Entity` table must include
//! columns matching the fields this encoder emits (`entity_id`, `x`, `y`, `z`, `user_data`, `cluster_id`).
//!
//! **Progressive-API note (see `docs/architecture/progressive-api.md`):** this is the level-1
//! auto-persist path. Level-0 users get positions for free; level-1 users put any additional
//! per-entity state in `EntityStateEntry::user_data` and it rides along in the same snapshot.
//! Level-2+ (explicit flush trigger, custom reducer, typed schemas) is deferred until a real
//! game-driven use case demands it.
//!
//! This module does not own simulation timing; `cluster_runner` decides when to call it.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use arcane_core::replication_channel::EntityStateEntry;
use uuid::Uuid;

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

/// Parse SpacetimeDB Entity table response, filtering by cluster_ids.
/// Expected schema: array of {entity_id: {__uuid__: number}, x, y, z, user_data, cluster_id, ...}
fn parse_entity_rows(value: serde_json::Value, cluster_ids: &[Uuid]) -> Vec<EntityStateEntry> {
    let cluster_id_set: HashSet<_> = cluster_ids.iter().copied().collect();
    let mut entries = Vec::new();

    if let Some(rows) = value.as_array() {
        for row in rows {
            let entity_id = match extract_uuid(&row["entity_id"]) {
                Some(id) => id,
                None => continue,
            };
            let cluster_id = match extract_uuid(&row["cluster_id"]) {
                Some(id) => id,
                None => continue,
            };
            if !cluster_id_set.contains(&cluster_id) {
                continue;
            }
            let x = row["x"].as_f64().unwrap_or(0.0);
            let y = row["y"].as_f64().unwrap_or(0.0);
            let z = row["z"].as_f64().unwrap_or(0.0);
            let user_data = if let Some(ud_str) = row["user_data"].as_str() {
                if ud_str.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_str(ud_str).unwrap_or(serde_json::Value::Null)
                }
            } else {
                serde_json::Value::Null
            };

            let mut entry = EntityStateEntry::new(
                entity_id,
                cluster_id,
                arcane_core::Vec3::new(x, y, z),
                arcane_core::Vec3::new(0.0, 0.0, 0.0), // velocity is not persisted
            );
            entry.user_data = user_data;
            entries.push(entry);
        }
    }
    entries
}

/// Extract UUID from SpacetimeDB's {__uuid__: u128} format.
fn extract_uuid(value: &serde_json::Value) -> Option<Uuid> {
    value
        .get("__uuid__")
        .and_then(|v| v.as_u64())
        .map(|n| Uuid::from_u128(n as u128))
}

pub struct SpacetimeDbPersist {
    client: reqwest::blocking::Client,
    url: String,
    interval_ticks: u64,
    /// Max entities per HTTP request (0 = no cap, send all in one request).
    max_batch_size: usize,
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
        Some(Self {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("reqwest client"),
            url,
            interval_ticks,
            max_batch_size,
        })
    }

    pub fn maybe_persist(&self, tick: u64, entries: &[EntityStateEntry]) {
        if !should_persist_tick(tick, self.interval_ticks, entries.len()) {
            return;
        }
        let chunk_size = if self.max_batch_size > 0 {
            self.max_batch_size
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
            match self
                .client
                .post(&self.url)
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
        if tick.is_multiple_of(self.interval_ticks * 10) && !entries.is_empty() {
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

    /// Read entity snapshots from SpacetimeDB for the given cluster IDs (e.g., neighbor clusters).
    /// Returns a flat list of EntityStateEntry for all matches.
    /// If SpacetimeDB is unreachable or empty, returns an empty list (soft failure).
    pub fn read_entities_for_clusters(
        uri: Option<&str>,
        db: Option<&str>,
        cluster_ids: &[Uuid],
    ) -> Vec<EntityStateEntry> {
        if cluster_ids.is_empty() {
            return Vec::new();
        }
        let uri = uri.unwrap_or_else(|| "http://127.0.0.1:3000");
        let db = db.unwrap_or_else(|| "arcane");
        let url = format!(
            "{}/v1/database/{}/tables/Entity",
            uri.trim_end_matches('/'),
            db
        );

        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("SpacetimeDB read: client build failed: {}", e);
                return Vec::new();
            }
        };

        match client.get(&url).send() {
            Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>() {
                Ok(rows) => parse_entity_rows(rows, cluster_ids),
                Err(e) => {
                    eprintln!("SpacetimeDB read: parse failed: {}", e);
                    Vec::new()
                }
            },
            Ok(resp) => {
                eprintln!("SpacetimeDB read: HTTP {}", resp.status());
                Vec::new()
            }
            Err(e) => {
                eprintln!("SpacetimeDB read: request failed: {}", e);
                Vec::new()
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
}
