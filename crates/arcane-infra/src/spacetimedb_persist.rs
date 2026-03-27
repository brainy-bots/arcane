//! SpacetimeDB persistence adapter for throttled state snapshots.
//!
//! Responsibilities:
//! - derive persist cadence and endpoint config from environment
//! - encode `EntityStateEntry` batches into SpacetimeDB reducer payload shape
//! - send chunked HTTP requests and log success/failure totals
//!
//! This module does not own simulation timing; `cluster_runner` decides when to call it.

use std::time::{Duration, Instant};

use arcane_core::replication_channel::EntityStateEntry;

const TICK_RATE_HZ: u64 = 20;

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
}

fn encode_spacetimedb_entities_body(
    chunk: &[EntityStateEntry],
) -> Result<String, serde_json::Error> {
    // SpacetimeDB Entity table is position-only (entity_id, x, y, z); omit velocity.
    let entities: Vec<SpacetimeEntityRow> = chunk
        .iter()
        .map(|e| SpacetimeEntityRow {
            entity_id: SpacetimeUuid {
                __uuid__: u128::from_be_bytes(*e.entity_id.as_bytes()),
            },
            x: e.position.x,
            y: e.position.y,
            z: e.position.z,
        })
        .collect();
    serde_json::to_string(&vec![entities])
}

fn should_persist_tick(tick: u64, interval_ticks: u64, entries_len: usize) -> bool {
    tick.is_multiple_of(interval_ticks) && entries_len > 0
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
        let interval_ticks = (TICK_RATE_HZ / hz.max(1)).max(1);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_core::replication_channel::EntityStateEntry;
    use arcane_core::Vec3;
    use uuid::Uuid;

    fn mk_entry(id: Uuid, x: f64, y: f64, z: f64) -> EntityStateEntry {
        EntityStateEntry {
            entity_id: id,
            cluster_id: Uuid::nil(),
            position: Vec3::new(x, y, z),
            velocity: Vec3::new(9.0, 9.0, 9.0),
        }
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
    fn should_persist_tick_obeys_cadence_and_non_empty_entries() {
        assert!(should_persist_tick(0, 20, 1));
        assert!(should_persist_tick(20, 20, 5));
        assert!(!should_persist_tick(1, 20, 5));
        assert!(!should_persist_tick(19, 20, 5));
        assert!(!should_persist_tick(20, 20, 0));
    }
}
