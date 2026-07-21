//! The routing table (#289 follow-up): the manager's decision output as a
//! readable Redis record, designed so a router worker serves any cluster job
//! with a bounded, batchable number of reads.
//!
//! ## Tables
//!
//! | key | writer → reader | content |
//! |---|---|---|
//! | `arcane:routing:<cluster>` | manager → router workers | complete per-cluster routing statement |
//! | `arcane:ownership` | manager → anyone | global entity→owner record (inspectability, bootstrap, future relay session table) |
//! | `arcane:state:<cluster>` | nodes → router/manager | live entity state (see [`crate::state_keys`]) |
//! | `arcane:inbox:<cluster>` | router → node | composed frames (see [`crate::node_inbox`]) |
//!
//! ## The routing doc is a complete statement
//!
//! Like the #289 inbox frames, each doc is idempotent and self-sufficient:
//! the cluster's full owned set, its full interest candidate list (with the
//! predictor's `p` and the replication gate's force marks), and the flip
//! events affecting it. Plain SET, replaced every manager cycle — a record,
//! never a delta. Any worker can compute any cluster's frame from its doc
//! plus the referenced state docs; workers are stateless and interchangeable
//! (router scaling is independent of cluster topology).
//!
//! ## Query pattern (why reads are bounded)
//!
//! A worker pass over jobs `C1..Ck`:
//!
//! 1. `MGET routing:C1 .. routing:Ck` — ONE round trip. Each doc lists the
//!    owner cluster of every interest candidate, so the worker now knows
//!    exactly which state docs it needs.
//! 2. `MGET state:<distinct referenced owners>` — ONE round trip, batched
//!    across all k jobs.
//!
//! **Two round trips per pass, regardless of job count.** To reach one:
//! either merge both MGETs (fetch all state docs — fine at tens of clusters)
//! or a server-side Lua `EVAL` that chases the references (minimal bytes at
//! scale). Both are drop-ins behind [`RoutingTable`]; not built until needed.
//!
//! The in-process v1 router pass reads THROUGH this table (write → read-back
//! → route), so splitting workers out later is pure process topology.

use crate::ownership_migration::OwnershipFlip;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// One interest candidate in a cluster's routing doc: an entity some owned
/// entity of this cluster has a non-zero predicted interaction with.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InterestEntry {
    pub entity_id: Uuid,
    /// The candidate's owning cluster at doc-build time. This is what makes
    /// the state join one bounded MGET: the doc tells the worker exactly
    /// which state docs it needs.
    pub owner: Uuid,
    /// Predictor output for the strongest edge to this candidate (dedup by
    /// max p at build time). The router evaluates the RATE LAW against p —
    /// tier assignment stays router-side per the architecture.
    pub p: f64,
    /// Replication-gate force-include: deliver at Full tier regardless of
    /// the rate law (pending-flip warm-up, §8).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forced: bool,
}

/// One cluster's complete routing statement. Written by the manager every
/// cycle to `arcane:routing:<cluster>`; read whole by router workers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RoutingDoc {
    /// Manager cycle tick at write time (staleness detection).
    pub tick: u64,
    /// The cluster's COMPLETE owned set (#289 statement; ids only — the
    /// node already has state for what it simulates).
    pub owned: Vec<Uuid>,
    /// Complete interest candidate list for this cluster.
    pub interest: Vec<InterestEntry>,
    /// Flip events affecting this cluster (gate ordering / observability).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flips: Vec<OwnershipFlip>,
}

/// Redis key for a cluster's routing doc.
pub fn routing_key(cluster_id: Uuid) -> String {
    format!("arcane:routing:{}", cluster_id.hyphenated())
}

/// Redis key for the global ownership record.
pub const OWNERSHIP_KEY: &str = "arcane:ownership";

/// The routing table: manager writes per-cluster docs (+ the derived global
/// ownership record); router workers read their jobs' docs in one batch.
/// Sans-IO seam — the manager/router logic stays transport-agnostic.
/// Implementations: in-memory (deterministic tests) and Redis (production).
pub trait RoutingTable: Send {
    /// Write all docs for this cycle (one batched round trip in the Redis
    /// impl) and refresh the global ownership record derived from them.
    fn write(&mut self, docs: &[(Uuid, RoutingDoc)]) -> Result<(), String>;
    /// Read the docs for a set of clusters (one batched round trip in the
    /// Redis impl). Missing clusters yield no entry.
    fn read(&mut self, clusters: &[Uuid]) -> Result<Vec<(Uuid, RoutingDoc)>, String>;
    /// Read the global ownership record (entity → owning cluster).
    fn read_ownership(&mut self) -> Result<HashMap<Uuid, Uuid>, String>;
}

/// In-memory routing table for deterministic tests.
#[derive(Default)]
pub struct InMemoryRoutingTable {
    docs: Mutex<HashMap<Uuid, RoutingDoc>>,
    ownership: Mutex<HashMap<Uuid, Uuid>>,
}

impl InMemoryRoutingTable {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RoutingTable for InMemoryRoutingTable {
    fn write(&mut self, docs: &[(Uuid, RoutingDoc)]) -> Result<(), String> {
        let mut d = self.docs.lock().unwrap();
        let mut o = self.ownership.lock().unwrap();
        o.clear();
        for (cluster, doc) in docs {
            for e in &doc.owned {
                o.insert(*e, *cluster);
            }
            d.insert(*cluster, doc.clone());
        }
        Ok(())
    }

    fn read(&mut self, clusters: &[Uuid]) -> Result<Vec<(Uuid, RoutingDoc)>, String> {
        let d = self.docs.lock().unwrap();
        Ok(clusters
            .iter()
            .filter_map(|c| d.get(c).map(|doc| (*c, doc.clone())))
            .collect())
    }

    fn read_ownership(&mut self) -> Result<HashMap<Uuid, Uuid>, String> {
        Ok(self.ownership.lock().unwrap().clone())
    }
}

/// Redis-backed routing table. Synchronous (the manager cycle is off the hot
/// path and tolerates a failed cycle); reconnects lazily on error.
pub struct RedisRoutingTable {
    client: redis::Client,
    conn: Option<redis::Connection>,
}

impl RedisRoutingTable {
    pub fn new(redis_url: &str) -> Result<Self, String> {
        let client =
            redis::Client::open(redis_url).map_err(|e| format!("Redis open failed: {}", e))?;
        Ok(Self { client, conn: None })
    }

    fn conn(&mut self) -> Result<&mut redis::Connection, String> {
        if self.conn.is_none() {
            self.conn = Some(
                self.client
                    .get_connection()
                    .map_err(|e| format!("Redis connect failed: {}", e))?,
            );
        }
        Ok(self.conn.as_mut().unwrap())
    }
}

impl RoutingTable for RedisRoutingTable {
    fn write(&mut self, docs: &[(Uuid, RoutingDoc)]) -> Result<(), String> {
        if docs.is_empty() {
            return Ok(());
        }
        // One MSET for all routing docs + the derived ownership record:
        // a single atomic round trip per cycle.
        let mut ownership: HashMap<String, String> = HashMap::new();
        let mut cmd = redis::cmd("MSET");
        for (cluster, doc) in docs {
            let payload =
                serde_json::to_string(doc).map_err(|e| format!("encode routing doc: {}", e))?;
            cmd.arg(routing_key(*cluster)).arg(payload);
            for e in &doc.owned {
                ownership.insert(e.hyphenated().to_string(), cluster.hyphenated().to_string());
            }
        }
        let ownership_json = serde_json::to_string(&ownership)
            .map_err(|e| format!("encode ownership record: {}", e))?;
        cmd.arg(OWNERSHIP_KEY).arg(ownership_json);

        let res: Result<(), redis::RedisError> = cmd.query(self.conn()?);
        if let Err(e) = res {
            self.conn = None;
            return Err(format!("routing table MSET failed: {}", e));
        }
        Ok(())
    }

    fn read(&mut self, clusters: &[Uuid]) -> Result<Vec<(Uuid, RoutingDoc)>, String> {
        if clusters.is_empty() {
            return Ok(Vec::new());
        }
        // ONE MGET round trip for any number of cluster jobs.
        let mut cmd = redis::cmd("MGET");
        for c in clusters {
            cmd.arg(routing_key(*c));
        }
        let res: Result<Vec<Option<String>>, redis::RedisError> = cmd.query(self.conn()?);
        match res {
            Ok(values) => Ok(clusters
                .iter()
                .zip(values)
                .filter_map(|(c, v)| {
                    v.and_then(|s| serde_json::from_str::<RoutingDoc>(&s).ok())
                        .map(|doc| (*c, doc))
                })
                .collect()),
            Err(e) => {
                self.conn = None;
                Err(format!("routing table MGET failed: {}", e))
            }
        }
    }

    fn read_ownership(&mut self) -> Result<HashMap<Uuid, Uuid>, String> {
        let res: Result<Option<String>, redis::RedisError> =
            redis::cmd("GET").arg(OWNERSHIP_KEY).query(self.conn()?);
        match res {
            Ok(Some(s)) => {
                let raw: HashMap<String, String> =
                    serde_json::from_str(&s).map_err(|e| format!("decode ownership: {}", e))?;
                Ok(raw
                    .into_iter()
                    .filter_map(|(k, v)| {
                        Some((Uuid::parse_str(&k).ok()?, Uuid::parse_str(&v).ok()?))
                    })
                    .collect())
            }
            Ok(None) => Ok(HashMap::new()),
            Err(e) => {
                self.conn = None;
                Err(format!("ownership GET failed: {}", e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(i: u8) -> Uuid {
        Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
    }

    fn doc(tick: u64, owned: Vec<Uuid>, interest: Vec<InterestEntry>) -> RoutingDoc {
        RoutingDoc {
            tick,
            owned,
            interest,
            flips: vec![],
        }
    }

    #[test]
    fn routing_doc_roundtrips_through_json() {
        let d = RoutingDoc {
            tick: 7,
            owned: vec![uuid(1), uuid(2)],
            interest: vec![InterestEntry {
                entity_id: uuid(3),
                owner: uuid(9),
                p: 0.75,
                forced: true,
            }],
            flips: vec![OwnershipFlip {
                entity_id: uuid(3),
                from_cluster: uuid(9),
                to_cluster: uuid(8),
                effective_tick: 40,
            }],
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: RoutingDoc = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn inmemory_write_read_and_ownership_record() {
        let mut table = InMemoryRoutingTable::new();
        let c1 = uuid(1);
        let c2 = uuid(2);
        table
            .write(&[
                (c1, doc(1, vec![uuid(10), uuid(11)], vec![])),
                (c2, doc(1, vec![uuid(20)], vec![])),
            ])
            .unwrap();

        let read = table.read(&[c1, c2, uuid(3)]).unwrap();
        assert_eq!(read.len(), 2, "unknown cluster yields no entry");
        assert_eq!(read[0].1.owned, vec![uuid(10), uuid(11)]);

        let own = table.read_ownership().unwrap();
        assert_eq!(own.get(&uuid(10)), Some(&c1));
        assert_eq!(own.get(&uuid(20)), Some(&c2));
        assert_eq!(own.len(), 3);
    }

    #[test]
    fn write_replaces_the_record_wholesale() {
        // The table is a record, not a fold: a rewrite drops entities that
        // disappeared (despawns don't linger in the ownership record).
        let mut table = InMemoryRoutingTable::new();
        let c1 = uuid(1);
        table
            .write(&[(c1, doc(1, vec![uuid(10)], vec![]))])
            .unwrap();
        table
            .write(&[(c1, doc(2, vec![uuid(11)], vec![]))])
            .unwrap();
        let own = table.read_ownership().unwrap();
        assert!(!own.contains_key(&uuid(10)), "despawned entity dropped");
        assert_eq!(own.get(&uuid(11)), Some(&c1));
    }

    /// Redis integration: requires a local Redis; skips silently otherwise
    /// (same pattern as state_keys::redis_integration_publish_and_fetch).
    #[test]
    fn redis_roundtrip_when_available() {
        let mut table = match RedisRoutingTable::new("redis://127.0.0.1:6379") {
            Ok(t) => t,
            Err(_) => return,
        };
        let c1 = Uuid::new_v4();
        let e1 = Uuid::new_v4();
        let d = doc(
            42,
            vec![e1],
            vec![InterestEntry {
                entity_id: Uuid::new_v4(),
                owner: Uuid::new_v4(),
                p: 0.5,
                forced: false,
            }],
        );
        if table.write(&[(c1, d.clone())]).is_err() {
            return; // no server listening
        }
        let read = table.read(&[c1]).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].1, d);
        let own = table.read_ownership().unwrap();
        assert_eq!(own.get(&e1), Some(&c1));
    }
}
