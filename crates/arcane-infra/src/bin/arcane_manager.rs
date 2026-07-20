//! Manager HTTP service and control loop.
//!
//! The Manager runs the full control loop: fetches entity state from Redis, evaluates
//! the affinity model, routes flips to clusters via RedisInboxBus, and serves /join
//! requests with configurable assignment policies.
//!
//! Env contract (documented at startup):
//!   MANAGER_CLUSTERS — REQUIRED: comma-separated `cluster_id:host:port` entries
//!     (e.g., "uuid1:127.0.0.1:8080,uuid2:127.0.0.1:8082"). The bootstrap topology.
//!   MANAGER_HTTP_PORT — optional; default 8081.
//!   REDIS_URL — optional; default "redis://127.0.0.1:6379".
//!   MANAGER_CADENCE_MS — optional; default 1000. Control loop cycle interval.
//!   MANAGER_JOIN_POLICY — optional; default "least-loaded". Join policy:
//!     "least-loaded" (default): cluster with fewest owned entities; tie → registration order.
//!     "first-cluster": always first registered cluster.
//!     "round-robin": legacy counter-based round-robin.
//!   MANAGER_TARGET_CLUSTERS — optional; partition into at most N clusters. Parsed but NOT
//!     wired in v1 (TODO: integrate with partitioner; currently no-op).
//!   MANAGER_CAPACITY_FACTOR — optional float; default uses AffinityConfig default (~1.5).
//!   MANAGER_STALE_LIMIT_MS — optional; default 3 * cadence. Staleness window for clusters.

use std::collections::{HashMap, HashSet};
use std::env;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use arcane_infra::manager_runtime::ManagerRuntime;
use arcane_infra::node_inbox::RedisInboxBus;
use arcane_infra::router_core::RouterConfig;
use arcane_infra::state_keys::RedisStateSource;
use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use tokio::time::sleep;
use uuid::Uuid;

#[derive(Clone, Serialize)]
struct JoinResponse {
    cluster_id: String,
    server_host: String,
    server_port: u16,
}

/// Parsed cluster registration.
#[derive(Clone, Debug)]
struct ClusterReg {
    id: Uuid,
    host: String,
    port: u16,
}

/// Join policy selection.
#[derive(Clone, Debug, PartialEq)]
enum JoinPolicy {
    LeastLoaded,
    FirstCluster,
    RoundRobin,
}

/// Shared state: assignments, stale clusters, registered order.
/// Refreshed each control cycle; accessed by /join handler.
#[derive(Clone, Debug)]
struct JoinState {
    assignments: HashMap<Uuid, Uuid>,
    stale_clusters: HashSet<Uuid>,
    registration_order: Vec<Uuid>,
}

/// Handler state: clusters, policy, join state, round-robin counter.
#[derive(Clone)]
struct ManagerState {
    clusters: Vec<ClusterReg>,
    policy: JoinPolicy,
    join_state: Arc<Mutex<JoinState>>,
    rr_counter: Arc<AtomicUsize>,
}

fn parse_clusters(s: &str) -> Result<Vec<ClusterReg>, String> {
    let mut clusters = Vec::new();
    for entry in s.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry.splitn(3, ':').collect();
        if parts.len() != 3 {
            return Err(format!(
                "Invalid cluster entry (expected id:host:port): {}",
                entry
            ));
        }
        let id = Uuid::parse_str(parts[0]).map_err(|e| format!("Invalid cluster ID: {}", e))?;
        let port: u16 = parts[2]
            .parse()
            .map_err(|e| format!("Invalid port: {}", e))?;
        clusters.push(ClusterReg {
            id,
            host: parts[1].to_string(),
            port,
        });
    }
    if clusters.is_empty() {
        return Err("No clusters parsed from MANAGER_CLUSTERS".to_string());
    }
    Ok(clusters)
}

fn parse_join_policy(s: &str) -> JoinPolicy {
    match s.to_lowercase().as_str() {
        "first-cluster" => JoinPolicy::FirstCluster,
        "round-robin" => JoinPolicy::RoundRobin,
        _ => JoinPolicy::LeastLoaded, // default
    }
}

/// Select join cluster based on policy.
fn select_join_cluster(
    policy: &JoinPolicy,
    join_state: &JoinState,
    _clusters: &[ClusterReg],
    rr_counter: &AtomicUsize,
) -> Option<Uuid> {
    match policy {
        JoinPolicy::FirstCluster => join_state.registration_order.first().and_then(|&id| {
            if join_state.stale_clusters.contains(&id) {
                None
            } else {
                Some(id)
            }
        }),
        JoinPolicy::RoundRobin => {
            let live_clusters: Vec<Uuid> = join_state
                .registration_order
                .iter()
                .filter(|id| !join_state.stale_clusters.contains(id))
                .copied()
                .collect();
            if live_clusters.is_empty() {
                None
            } else {
                let idx = rr_counter.fetch_add(1, Ordering::Relaxed) % live_clusters.len();
                Some(live_clusters[idx])
            }
        }
        JoinPolicy::LeastLoaded => {
            let mut best_cluster: Option<(Uuid, usize)> = None;
            for &cluster_id in &join_state.registration_order {
                if join_state.stale_clusters.contains(&cluster_id) {
                    continue;
                }
                let count = join_state
                    .assignments
                    .values()
                    .filter(|&&c| c == cluster_id)
                    .count();
                match best_cluster {
                    None => best_cluster = Some((cluster_id, count)),
                    Some((_, best_count)) => {
                        if count < best_count {
                            best_cluster = Some((cluster_id, count));
                        }
                    }
                }
            }
            best_cluster.map(|(id, _)| id)
        }
    }
}

async fn join_handler(State(s): State<ManagerState>) -> Json<JoinResponse> {
    let join_state = s.join_state.lock().unwrap();
    let cluster_id = select_join_cluster(&s.policy, &join_state, &s.clusters, &s.rr_counter)
        .and_then(|id| s.clusters.iter().find(|c| c.id == id).cloned())
        .unwrap_or_else(|| s.clusters[0].clone());
    drop(join_state);

    Json(JoinResponse {
        cluster_id: cluster_id.id.to_string(),
        server_host: cluster_id.host,
        server_port: cluster_id.port,
    })
}

/// Tick-based staleness tracker: a cluster is stale when its published tick has not
/// advanced within the stale window. An EMPTY cluster that keeps publishing (warm
/// spare: advancing tick, zero entities) is NOT stale — "no entities = stale" would
/// block flips to warm spares and deadlock the spread-from-one-cluster regime.
/// A cluster never seen at all is stale (it has not proven liveness yet).
struct StaleTracker {
    /// cluster -> (last observed tick, instant when that tick was first observed)
    seen: HashMap<Uuid, (u64, std::time::Instant)>,
}

impl StaleTracker {
    fn new() -> Self {
        Self {
            seen: HashMap::new(),
        }
    }

    /// Feed the latest (cluster, tick) observations; returns the stale set.
    fn update(
        &mut self,
        docs: &[(Uuid, u64)],
        registered: &[Uuid],
        stale_limit: Duration,
        now: std::time::Instant,
    ) -> HashSet<Uuid> {
        for (cluster, tick) in docs {
            match self.seen.get(cluster) {
                Some((last_tick, _)) if last_tick == tick => {} // not advancing
                _ => {
                    self.seen.insert(*cluster, (*tick, now));
                }
            }
        }
        registered
            .iter()
            .filter(|c| match self.seen.get(c) {
                None => true, // never published
                Some((_, since)) => now.duration_since(*since) > stale_limit,
            })
            .copied()
            .collect()
    }
}

/// Control loop: fetches state, updates runtime, detects staleness, runs cycles.
async fn control_loop(
    clusters: Vec<ClusterReg>,
    redis_url: String,
    cadence_ms: u64,
    capacity_factor: Option<f64>,
    stale_limit_ms: u64,
    join_state: Arc<Mutex<JoinState>>,
) {
    let cluster_ids: Vec<Uuid> = clusters.iter().map(|c| c.id).collect();
    let mut cycle_count = 0u64;
    let stale_limit = Duration::from_millis(stale_limit_ms);

    loop {
        // Try to build/rebuild runtime and state source each cycle.
        let state_source = match RedisStateSource::new(&redis_url, cluster_ids.clone()) {
            Ok(ss) => ss,
            Err(e) => {
                eprintln!("arcane-manager: Failed to create state source: {}", e);
                sleep(Duration::from_millis(cadence_ms)).await;
                continue;
            }
        };

        let bus = match RedisInboxBus::new(&redis_url) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("arcane-manager: Failed to create inbox bus: {}", e);
                sleep(Duration::from_millis(cadence_ms)).await;
                continue;
            }
        };

        let mut manager = ArcaneManager::with_model("affinity");

        // Apply operator config: capacity factor and/or pin feature.
        // MANAGER_PIN_FEATURE names the game-declared feature that anchors an
        // entity to its current cluster (nonzero value = never migrate). The
        // v1 stand-in for CLUSTER_REASSIGN: client-driven entities stay on the
        // cluster their WS connection terminates at.
        let pin_feature = env::var("MANAGER_PIN_FEATURE")
            .ok()
            .filter(|s| !s.is_empty());
        if capacity_factor.is_some() || pin_feature.is_some() {
            let mut config = arcane_affinity::config::AffinityConfig {
                pin_feature: pin_feature.clone(),
                ..Default::default()
            };
            if let Some(factor) = capacity_factor {
                config.capacity_factor = factor;
            }
            manager.set_affinity_config(config);
        }
        if let Some(ref pf) = pin_feature {
            eprintln!("arcane-manager: pin feature '{pf}' — pinned entities never migrate");
        }

        let router_config = RouterConfig::default();
        let mut runtime = ManagerRuntime::new(manager, bus, router_config);
        // Routing table: the manager's decision output as a readable Redis
        // record (arcane:routing:<cluster> + arcane:ownership). The in-process
        // router pass reads THROUGH it, so splitting router workers out later
        // is pure process topology.
        match arcane_infra::routing_table::RedisRoutingTable::new(&redis_url) {
            Ok(table) => {
                runtime.set_routing_table(Box::new(table));
                eprintln!("arcane-manager: routing table on Redis (arcane:routing:*)");
            }
            Err(e) => eprintln!(
                "arcane-manager: routing table init failed ({}); using in-memory (frames still flow)",
                e
            ),
        }
        // Warm spares count as partitions: without this, an everyone-on-one-cluster
        // world has k=1 and can never spread.
        runtime.set_known_clusters(cluster_ids.clone());
        let mut stale_tracker = StaleTracker::new();
        let mut last_stale: HashSet<Uuid> = HashSet::new();

        loop {
            // a. Fetch entities from all cluster state keys.
            let records = state_source.fetch_all();

            // b. Update runtime with entities, velocities, features.
            for record in &records {
                let pos = Vec3::new(record.position.x, 0.0, record.position.y);
                let vel = Vec3::new(record.velocity.x, 0.0, record.velocity.y);
                runtime.update_entity(record.entity_id, record.cluster_id, pos);
                runtime.set_entity_velocity(record.entity_id, vel);
                for (name, value) in &record.features {
                    runtime.set_entity_feature(record.entity_id, name, *value);
                }
            }

            // c. Staleness check: detect clusters whose ticks haven't advanced.
            // Tick-based (ADR-005 Decision 3 guard): a cluster is stale when its
            // published tick stops advancing for stale_limit; empty-but-publishing
            // warm spares stay live.
            let docs = state_source.last_docs();
            let stale_clusters =
                stale_tracker.update(&docs, &cluster_ids, stale_limit, std::time::Instant::now());
            if stale_clusters != last_stale {
                eprintln!(
                    "arcane-manager: stale set changed: {:?}",
                    stale_clusters.iter().collect::<Vec<_>>()
                );
                last_stale = stale_clusters.clone();
            }

            // d. Block stale destinations and run cycle.
            runtime.set_blocked_destinations(stale_clusters.clone());
            let cycle_result = runtime.run_cycle();

            // e. Update join state with current assignments.
            let assignments = runtime.assignments().clone();
            let registration_order: Vec<Uuid> = cluster_ids.clone();
            {
                let mut js = join_state.lock().unwrap();
                js.assignments = assignments;
                js.stale_clusters = stale_clusters;
                js.registration_order = registration_order;
            }

            // f. Log cycle summary every N cycles.
            cycle_count += 1;
            #[allow(clippy::manual_is_multiple_of)]
            if cycle_count % 10 == 0 {
                match cycle_result {
                    Ok(report) => {
                        eprintln!(
                            "arcane-manager: cycle {} — entities={}, clusters={}, pending={}, published={}, frames={}",
                            cycle_count,
                            records.len(),
                            cluster_ids.len(),
                            report.pending_flips,
                            report.published_flips,
                            report.frames_published
                        );
                    }
                    Err(e) => {
                        eprintln!("arcane-manager: cycle {} error: {}", cycle_count, e);
                    }
                }
            }

            sleep(Duration::from_millis(cadence_ms)).await;
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let clusters_str = env::var("MANAGER_CLUSTERS").map_err(|_| {
        "MANAGER_CLUSTERS env var is REQUIRED (format: cluster_id:host:port,...)".to_string()
    })?;

    let clusters = parse_clusters(&clusters_str)?;

    let http_port: u16 = env::var("MANAGER_HTTP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8081);

    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let cadence_ms = env::var("MANAGER_CADENCE_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    let policy = env::var("MANAGER_JOIN_POLICY")
        .map(|s| parse_join_policy(&s))
        .unwrap_or(JoinPolicy::LeastLoaded);

    let capacity_factor = env::var("MANAGER_CAPACITY_FACTOR")
        .ok()
        .and_then(|s| s.parse().ok());

    let stale_limit_ms = env::var("MANAGER_STALE_LIMIT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3 * cadence_ms);

    // MANAGER_TARGET_CLUSTERS: parsed but NOT wired in v1.
    let _target_clusters = env::var("MANAGER_TARGET_CLUSTERS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok());
    if _target_clusters.is_some() {
        eprintln!("arcane-manager: MANAGER_TARGET_CLUSTERS parsed but not yet wired (TODO)");
    }

    eprintln!(
        "arcane-manager: started — {} cluster(s), policy={:?}, cadence={}ms, redis={}",
        clusters.len(),
        policy,
        cadence_ms,
        redis_url
    );

    // Initialize join state.
    let join_state = Arc::new(Mutex::new(JoinState {
        assignments: HashMap::new(),
        stale_clusters: HashSet::new(),
        registration_order: clusters.iter().map(|c| c.id).collect(),
    }));

    // Spawn control loop in background.
    let loop_join_state = join_state.clone();
    let loop_clusters = clusters.clone();
    tokio::spawn(async move {
        control_loop(
            loop_clusters,
            redis_url,
            cadence_ms,
            capacity_factor,
            stale_limit_ms,
            loop_join_state,
        )
        .await
    });

    // Set up HTTP server.
    let state = ManagerState {
        clusters: clusters.clone(),
        policy,
        join_state,
        rr_counter: Arc::new(AtomicUsize::new(0)),
    };

    let app = Router::new()
        .route("/join", get(join_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    eprintln!("arcane-manager: listening on http://{}", addr);

    axum::serve(
        tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| e.to_string())?,
        app,
    )
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clusters_valid() {
        let input = "12345678-1234-5678-1234-567812345678:127.0.0.1:8080,\
                     87654321-4321-8765-4321-876543218765:127.0.0.1:8082";
        let clusters = parse_clusters(input).expect("parse failed");
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].port, 8080);
        assert_eq!(clusters[1].port, 8082);
        assert_eq!(clusters[0].host, "127.0.0.1");
    }

    #[test]
    fn parse_clusters_single() {
        let input = "12345678-1234-5678-1234-567812345678:localhost:9000";
        let clusters = parse_clusters(input).expect("parse failed");
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].port, 9000);
        assert_eq!(clusters[0].host, "localhost");
    }

    #[test]
    fn parse_clusters_with_whitespace() {
        let input = "  12345678-1234-5678-1234-567812345678:127.0.0.1:8080 , \
                     87654321-4321-8765-4321-876543218765:127.0.0.1:8082  ";
        let clusters = parse_clusters(input).expect("parse failed");
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn parse_clusters_invalid_format() {
        let input = "invalid:entry";
        let result = parse_clusters(input);
        assert!(result.is_err());
    }

    #[test]
    fn stale_tracker_semantics() {
        use std::time::{Duration, Instant};
        let c1 = Uuid::from_u128(1); // advancing ticks
        let c2 = Uuid::from_u128(2); // frozen tick
        let c3 = Uuid::from_u128(3); // never publishes
        let spare = Uuid::from_u128(4); // warm spare: advancing tick, would have zero entities
        let registered = vec![c1, c2, c3, spare];
        let limit = Duration::from_millis(300);

        let mut tracker = StaleTracker::new();
        let t0 = Instant::now();

        // First observation: everyone who published is fresh; c3 (never seen) is stale.
        let stale = tracker.update(&[(c1, 10), (c2, 5), (spare, 1)], &registered, limit, t0);
        assert!(stale.contains(&c3), "never-published cluster must be stale");
        assert!(!stale.contains(&c1) && !stale.contains(&c2) && !stale.contains(&spare));

        // 400ms later: c1 and spare advanced, c2 frozen at 5 → c2 goes stale; the
        // EMPTY-but-publishing spare stays live (the warm-spare guarantee).
        let t1 = t0 + Duration::from_millis(400);
        let stale = tracker.update(&[(c1, 22), (c2, 5), (spare, 2)], &registered, limit, t1);
        assert!(stale.contains(&c2), "frozen-tick cluster must be stale");
        assert!(stale.contains(&c3));
        assert!(!stale.contains(&c1), "advancing cluster must not be stale");
        assert!(
            !stale.contains(&spare),
            "publishing warm spare must not be stale"
        );

        // c2 resumes advancing → recovers.
        let t2 = t1 + Duration::from_millis(100);
        let stale = tracker.update(&[(c1, 30), (c2, 6), (spare, 3)], &registered, limit, t2);
        assert!(
            !stale.contains(&c2),
            "recovered cluster must clear staleness"
        );
    }

    #[test]
    fn parse_join_policy_least_loaded() {
        assert_eq!(parse_join_policy("least-loaded"), JoinPolicy::LeastLoaded);
        assert_eq!(parse_join_policy("LEAST-LOADED"), JoinPolicy::LeastLoaded);
    }

    #[test]
    fn parse_join_policy_first_cluster() {
        assert_eq!(parse_join_policy("first-cluster"), JoinPolicy::FirstCluster);
    }

    #[test]
    fn parse_join_policy_round_robin() {
        assert_eq!(parse_join_policy("round-robin"), JoinPolicy::RoundRobin);
    }

    #[test]
    fn parse_join_policy_default() {
        assert_eq!(parse_join_policy("unknown"), JoinPolicy::LeastLoaded);
    }

    #[test]
    fn select_join_cluster_least_loaded() {
        let mut assignments = HashMap::new();
        assignments.insert(Uuid::from_u128(1), Uuid::from_u128(10));
        assignments.insert(Uuid::from_u128(2), Uuid::from_u128(10));
        assignments.insert(Uuid::from_u128(3), Uuid::from_u128(20));

        let c1 = Uuid::from_u128(10);
        let c2 = Uuid::from_u128(20);

        let join_state = JoinState {
            assignments,
            stale_clusters: HashSet::new(),
            registration_order: vec![c1, c2],
        };

        let clusters = vec![
            ClusterReg {
                id: c1,
                host: "127.0.0.1".to_string(),
                port: 8080,
            },
            ClusterReg {
                id: c2,
                host: "127.0.0.1".to_string(),
                port: 8081,
            },
        ];

        let rr_counter = Arc::new(AtomicUsize::new(0));
        let selected = select_join_cluster(
            &JoinPolicy::LeastLoaded,
            &join_state,
            &clusters,
            &rr_counter,
        )
        .expect("select failed");

        assert_eq!(
            selected, c2,
            "least-loaded should pick cluster with fewest entities"
        );
    }

    #[test]
    fn select_join_cluster_least_loaded_tie() {
        let mut assignments = HashMap::new();
        assignments.insert(Uuid::from_u128(1), Uuid::from_u128(10));
        assignments.insert(Uuid::from_u128(2), Uuid::from_u128(20));

        let c1 = Uuid::from_u128(10);
        let c2 = Uuid::from_u128(20);

        let join_state = JoinState {
            assignments,
            stale_clusters: HashSet::new(),
            registration_order: vec![c1, c2],
        };

        let clusters = vec![
            ClusterReg {
                id: c1,
                host: "127.0.0.1".to_string(),
                port: 8080,
            },
            ClusterReg {
                id: c2,
                host: "127.0.0.1".to_string(),
                port: 8081,
            },
        ];

        let rr_counter = Arc::new(AtomicUsize::new(0));
        let selected = select_join_cluster(
            &JoinPolicy::LeastLoaded,
            &join_state,
            &clusters,
            &rr_counter,
        )
        .expect("select failed");

        assert_eq!(
            selected, c1,
            "least-loaded tie should pick registration order"
        );
    }

    #[test]
    fn select_join_cluster_first_cluster() {
        let c1 = Uuid::from_u128(10);
        let c2 = Uuid::from_u128(20);

        let join_state = JoinState {
            assignments: HashMap::new(),
            stale_clusters: HashSet::new(),
            registration_order: vec![c1, c2],
        };

        let clusters = vec![
            ClusterReg {
                id: c1,
                host: "127.0.0.1".to_string(),
                port: 8080,
            },
            ClusterReg {
                id: c2,
                host: "127.0.0.1".to_string(),
                port: 8081,
            },
        ];

        let rr_counter = Arc::new(AtomicUsize::new(0));
        let selected = select_join_cluster(
            &JoinPolicy::FirstCluster,
            &join_state,
            &clusters,
            &rr_counter,
        )
        .expect("select failed");

        assert_eq!(selected, c1);
    }

    #[test]
    fn select_join_cluster_round_robin() {
        let c1 = Uuid::from_u128(10);
        let c2 = Uuid::from_u128(20);

        let join_state = JoinState {
            assignments: HashMap::new(),
            stale_clusters: HashSet::new(),
            registration_order: vec![c1, c2],
        };

        let clusters = vec![
            ClusterReg {
                id: c1,
                host: "127.0.0.1".to_string(),
                port: 8080,
            },
            ClusterReg {
                id: c2,
                host: "127.0.0.1".to_string(),
                port: 8081,
            },
        ];

        let rr_counter = Arc::new(AtomicUsize::new(0));

        let s1 = select_join_cluster(&JoinPolicy::RoundRobin, &join_state, &clusters, &rr_counter)
            .expect("select failed");
        let s2 = select_join_cluster(&JoinPolicy::RoundRobin, &join_state, &clusters, &rr_counter)
            .expect("select failed");

        assert_eq!(s1, c1);
        assert_eq!(s2, c2);
    }

    #[test]
    fn select_join_cluster_excludes_stale() {
        let c1 = Uuid::from_u128(10);
        let c2 = Uuid::from_u128(20);

        let mut stale = HashSet::new();
        stale.insert(c1);

        let join_state = JoinState {
            assignments: HashMap::new(),
            stale_clusters: stale,
            registration_order: vec![c1, c2],
        };

        let clusters = vec![
            ClusterReg {
                id: c1,
                host: "127.0.0.1".to_string(),
                port: 8080,
            },
            ClusterReg {
                id: c2,
                host: "127.0.0.1".to_string(),
                port: 8081,
            },
        ];

        let rr_counter = Arc::new(AtomicUsize::new(0));

        let selected = select_join_cluster(
            &JoinPolicy::LeastLoaded,
            &join_state,
            &clusters,
            &rr_counter,
        )
        .expect("select failed");

        assert_eq!(selected, c2, "stale clusters should be excluded");
    }
}
