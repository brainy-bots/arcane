//! Redis state keys: cluster-owned entity state keyed by cluster ID.
//!
//! Each cluster publishes its owned entity state (spine kinematics + dynamic features)
//! and observed edges to `arcane:state:<cluster_id>` as JSON. The Manager pulls these
//! keys at its own cadence (never blocks on stale data); nodes publish non-blocking via
//! a producer thread.

use arcane_affinity::feature_map::{EntityRecord, IEntityStateSource};
use uuid::Uuid;

/// One cluster's published control-plane state: its owned entities (spine +
/// dynamic features) and the interaction edges it locally observed since the
/// last publish. Written to `arcane:state:<cluster_id>` as JSON (plain SET,
/// no pub/sub — the Manager pulls; design §2.5).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ClusterStateDoc {
    pub cluster_id: Uuid,
    /// Node tick at write time (staleness detection).
    pub tick: u64,
    pub entities: Vec<EntityRecord>,
    /// Locally observed interaction edges (a, b, weight) — e.g. contacts,
    /// game-action pairs. Empty in v1 node publishes (accumulation source
    /// stays manager-side proximity until nodes report edges).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_edges: Vec<(Uuid, Uuid, f64)>,
}

/// Redis key for a cluster's state document.
pub fn state_key(cluster_id: Uuid) -> String {
    format!("arcane:state:{}", cluster_id.hyphenated())
}

/// Encode ClusterStateDoc to JSON string.
pub fn encode(doc: &ClusterStateDoc) -> Result<String, String> {
    serde_json::to_string(doc).map_err(|e| format!("encode failed: {}", e))
}

/// Decode ClusterStateDoc from JSON string.
pub fn decode(s: &str) -> Result<ClusterStateDoc, String> {
    serde_json::from_str(s).map_err(|e| format!("decode failed: {}", e))
}

/// Message queued for the state publisher thread.
struct PublishMessage {
    cluster_id: Uuid,
    doc: ClusterStateDoc,
}

/// Publishes cluster state documents to Redis (non-blocking).
/// Publishing is non-blocking: docs are enqueued on a producer thread via mpsc,
/// which owns the Redis connection and drains the queue.
pub struct StatePublisher {
    tx: std::sync::mpsc::Sender<PublishMessage>,
}

impl StatePublisher {
    pub fn new(redis_url: &str) -> Result<Self, String> {
        let client =
            redis::Client::open(redis_url).map_err(|e| format!("Redis open failed: {}", e))?;
        let (tx, rx) = std::sync::mpsc::channel::<PublishMessage>();

        std::thread::spawn(move || {
            // Lazily (re)connect; never exit on connection failure so `publish()` can always enqueue.
            let mut conn: Option<redis::Connection> = client.get_connection().ok();
            while let Ok(msg) = rx.recv() {
                if conn.is_none() {
                    conn = client.get_connection().ok();
                }
                let Some(c) = conn.as_mut() else {
                    continue;
                };

                let key = state_key(msg.cluster_id);
                if let Ok(payload) = encode(&msg.doc) {
                    let res: Result<(), redis::RedisError> =
                        redis::cmd("SET").arg(&key).arg(&payload).query(c);
                    if res.is_err() {
                        conn = None;
                    }
                }
            }
        });

        Ok(Self { tx })
    }

    /// Enqueue a state doc for non-blocking publication.
    /// Returns immediately without waiting on Redis.
    pub fn publish(&self, doc: &ClusterStateDoc) -> Result<(), String> {
        self.tx
            .send(PublishMessage {
                cluster_id: doc.cluster_id,
                doc: doc.clone(),
            })
            .map_err(|_| "publisher thread dead".to_string())
    }
}

/// Pure merge logic for fetch results + cache. Tests this without Redis.
fn merge_fetch(
    results: Vec<(Uuid, Option<ClusterStateDoc>)>,
    cache: &mut std::collections::HashMap<Uuid, ClusterStateDoc>,
) -> Vec<EntityRecord> {
    let mut records = Vec::new();
    for (cluster_id, doc_opt) in results {
        let doc = match doc_opt {
            Some(doc) => {
                cache.insert(cluster_id, doc.clone());
                doc
            }
            None => match cache.get(&cluster_id) {
                Some(cached) => cached.clone(),
                None => continue,
            },
        };
        records.extend(doc.entities);
    }
    records
}

/// Pulls cluster state from Redis keys (pull-only, no pub/sub).
/// Implements `IEntityStateSource` for the Manager to fetch state.
pub struct RedisStateSource {
    redis_url: String,
    cluster_ids: Vec<Uuid>,
    cache: std::sync::Mutex<std::collections::HashMap<Uuid, ClusterStateDoc>>,
    last_observed_edges: std::sync::Mutex<Vec<(Uuid, Uuid, f64)>>,
}

impl RedisStateSource {
    /// Create a new RedisStateSource for the given cluster IDs.
    /// The bootstrap cluster list (from MANAGER_CLUSTERS env later; B3's job).
    pub fn new(redis_url: &str, cluster_ids: Vec<Uuid>) -> Result<Self, String> {
        // Validate Redis connectivity early.
        let client =
            redis::Client::open(redis_url).map_err(|e| format!("Redis open failed: {}", e))?;
        let _ = client
            .get_connection()
            .map_err(|e| format!("Redis connection failed: {}", e))?;

        Ok(Self {
            redis_url: redis_url.to_string(),
            cluster_ids,
            cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            last_observed_edges: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Fetch all entity records from cluster state keys.
    /// Returns concatenated records; uses cache on missing/error.
    pub fn fetch_all(&self) -> Vec<EntityRecord> {
        let client = match redis::Client::open(self.redis_url.as_str()) {
            Ok(c) => c,
            Err(_) => {
                // Connection error: fall back to cache
                let cache = self.cache.lock().unwrap();
                let records: Vec<EntityRecord> = cache
                    .values()
                    .flat_map(|doc| doc.entities.clone())
                    .collect();
                return records;
            }
        };

        let mut conn = match client.get_connection() {
            Ok(c) => c,
            Err(_) => {
                // Connection error: fall back to cache
                let cache = self.cache.lock().unwrap();
                let records: Vec<EntityRecord> = cache
                    .values()
                    .flat_map(|doc| doc.entities.clone())
                    .collect();
                return records;
            }
        };

        let mut results = Vec::new();
        let mut edges = Vec::new();

        for cluster_id in &self.cluster_ids {
            let key = state_key(*cluster_id);
            let payload: Result<String, redis::RedisError> =
                redis::cmd("GET").arg(&key).query(&mut conn);

            let doc_opt = match payload {
                Ok(p) => match decode(&p) {
                    Ok(doc) => {
                        edges.extend(doc.observed_edges.clone());
                        Some(doc)
                    }
                    Err(_) => None,
                },
                Err(_) => None,
            };

            results.push((*cluster_id, doc_opt));
        }

        let mut cache = self.cache.lock().unwrap();
        let records = merge_fetch(results, &mut cache);

        let mut last_edges = self.last_observed_edges.lock().unwrap();
        *last_edges = edges;

        records
    }

    /// Last observed edges from the latest fetch.
    pub fn last_observed_edges(&self) -> Vec<(Uuid, Uuid, f64)> {
        self.last_observed_edges.lock().unwrap().clone()
    }
}

impl IEntityStateSource for RedisStateSource {
    fn fetch_all(&self) -> Vec<EntityRecord> {
        RedisStateSource::fetch_all(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_affinity::feature_map::FeatureMap;
    use arcane_core::types::Vec2;

    #[test]
    fn state_key_format() {
        let cluster_id = Uuid::parse_str("12345678-1234-5678-1234-567812345678").unwrap();
        let key = state_key(cluster_id);
        assert_eq!(key, "arcane:state:12345678-1234-5678-1234-567812345678");
    }

    #[test]
    fn encode_decode_roundtrip_with_edges() {
        let mut features = FeatureMap::new();
        features.insert("squad".to_string(), 1.5);

        let record = EntityRecord {
            entity_id: Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap(),
            cluster_id: Uuid::parse_str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb").unwrap(),
            position: Vec2::new(10.0, 20.0),
            velocity: Vec2::new(1.0, 2.0),
            features,
        };

        let cluster_id = Uuid::parse_str("cccccccc-cccc-cccc-cccc-cccccccccccc").unwrap();

        let doc = ClusterStateDoc {
            cluster_id,
            tick: 42,
            entities: vec![record.clone()],
            observed_edges: vec![(Uuid::nil(), Uuid::max(), 0.5)],
        };

        let encoded = encode(&doc).expect("encode");
        let decoded = decode(&encoded).expect("decode");

        assert_eq!(decoded.cluster_id, cluster_id);
        assert_eq!(decoded.tick, 42);
        assert_eq!(decoded.entities.len(), 1);
        assert_eq!(decoded.entities[0], record);
        assert_eq!(decoded.observed_edges.len(), 1);
        assert_eq!(decoded.observed_edges[0].2, 0.5);
    }

    #[test]
    fn encode_decode_roundtrip_empty_features() {
        let record = EntityRecord {
            entity_id: Uuid::nil(),
            cluster_id: Uuid::nil(),
            position: Vec2::new(1.0, 2.0),
            velocity: Vec2::new(0.5, -0.5),
            features: FeatureMap::new(),
        };

        let doc = ClusterStateDoc {
            cluster_id: Uuid::nil(),
            tick: 100,
            entities: vec![record.clone()],
            observed_edges: vec![],
        };

        let encoded = encode(&doc).expect("encode");
        assert!(!encoded.contains("features"));

        let decoded = decode(&encoded).expect("decode");
        assert_eq!(decoded.entities[0], record);
    }

    #[test]
    fn merge_fetch_fresh_replaces_cache() {
        let mut cache = std::collections::HashMap::new();

        let cid1 = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let old_doc = ClusterStateDoc {
            cluster_id: cid1,
            tick: 1,
            entities: vec![],
            observed_edges: vec![],
        };
        cache.insert(cid1, old_doc);

        let record = EntityRecord {
            entity_id: Uuid::nil(),
            cluster_id: cid1,
            position: Vec2::new(1.0, 2.0),
            velocity: Vec2::new(0.0, 0.0),
            features: FeatureMap::new(),
        };

        let new_doc = ClusterStateDoc {
            cluster_id: cid1,
            tick: 2,
            entities: vec![record.clone()],
            observed_edges: vec![],
        };

        let results = vec![(cid1, Some(new_doc))];
        let records = merge_fetch(results, &mut cache);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0], record);
        assert_eq!(cache.get(&cid1).unwrap().tick, 2);
    }

    #[test]
    fn merge_fetch_missing_key_uses_cache() {
        let mut cache = std::collections::HashMap::new();

        let cid1 = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let cached_doc = ClusterStateDoc {
            cluster_id: cid1,
            tick: 1,
            entities: vec![EntityRecord {
                entity_id: Uuid::nil(),
                cluster_id: cid1,
                position: Vec2::new(1.0, 2.0),
                velocity: Vec2::new(0.0, 0.0),
                features: FeatureMap::new(),
            }],
            observed_edges: vec![],
        };
        cache.insert(cid1, cached_doc.clone());

        let results = vec![(cid1, None)];
        let records = merge_fetch(results, &mut cache);

        assert_eq!(records.len(), 1);
        assert_eq!(cache.get(&cid1).unwrap().tick, 1);
    }

    #[test]
    fn merge_fetch_never_seen_cluster_contributes_nothing() {
        let mut cache = std::collections::HashMap::new();

        let cid1 = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let results = vec![(cid1, None)];
        let records = merge_fetch(results, &mut cache);

        assert_eq!(records.len(), 0);
        assert!(!cache.contains_key(&cid1));
    }

    #[test]
    fn state_publisher_enqueues_nonblocking() {
        let publisher = match StatePublisher::new("redis://127.0.0.1:16379") {
            Ok(p) => p,
            Err(_) => return, // Skip if we can't create the publisher.
        };

        let doc = ClusterStateDoc {
            cluster_id: Uuid::nil(),
            tick: 1,
            entities: vec![],
            observed_edges: vec![],
        };

        let start = std::time::Instant::now();
        let result = publisher.publish(&doc);
        let elapsed = start.elapsed();

        // publish() should return promptly (< 10ms).
        assert!(
            elapsed.as_millis() < 10,
            "publish() took too long: {:?}",
            elapsed
        );
        // The enqueue itself should succeed (thread is running).
        assert!(result.is_ok(), "publish should enqueue successfully");
    }

    #[test]
    #[ignore]
    fn redis_integration_publish_and_fetch() {
        // This test requires Redis on 127.0.0.1:6379.
        // Run with: cargo test --test state_keys_integration -- --ignored --nocapture

        let redis_url = "redis://127.0.0.1:6379";

        let publisher = match StatePublisher::new(redis_url) {
            Ok(p) => p,
            Err(_) => {
                eprintln!("Skipping Redis integration test: Redis unavailable");
                return;
            }
        };

        let cluster_id = Uuid::parse_str("12345678-1234-5678-1234-567812345678").unwrap();

        let record = EntityRecord {
            entity_id: Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap(),
            cluster_id,
            position: Vec2::new(10.0, 20.0),
            velocity: Vec2::new(1.0, 2.0),
            features: FeatureMap::new(),
        };

        let doc = ClusterStateDoc {
            cluster_id,
            tick: 42,
            entities: vec![record.clone()],
            observed_edges: vec![(Uuid::nil(), Uuid::max(), 0.5)],
        };

        // Publish
        publisher.publish(&doc).expect("publish");

        // Give the publisher thread time to process
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Fetch
        let source = RedisStateSource::new(redis_url, vec![cluster_id]).expect("source");
        let records = source.fetch_all();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0], record);

        let edges = source.last_observed_edges();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].2, 0.5);
    }
}
