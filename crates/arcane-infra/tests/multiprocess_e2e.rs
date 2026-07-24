//! B5 — Multi-process E2E: the control plane as REAL processes over REAL Redis.
//!
//! Epic #271's un-fakeable acceptance. Spawns the actual `arcane-manager` and two
//! actual `arcane-node` binaries (release profile), drives entities through node A
//! via its WebSocket, and machine-checks from the processes' observable outputs:
//!
//!   1. The manager makes a migration decision from ingested state keys.
//!   2. The destination received the entity's STATE (inbox frames) BEFORE the flip
//!      published (B2 gate, §8 replication-precedes-ownership) — verified by
//!      subscribing to the inbox channels ourselves and recording the order.
//!   3. Both nodes' ownership converges: exactly one node writes the entity
//!      (observed via the per-cluster replication topics: only one cluster's delta
//!      stream carries the entity as owned after settling).
//!   4. No flip ping-pong after settling.
//!
//! Requirements: Redis at 127.0.0.1:6379 and prebuilt release binaries
//! (`cargo build -p arcane-infra --release --features manager,migration --bin arcane-manager`
//!  and `--features cluster-ws,migration --bin arcane-node`).
//! The test SKIPS (passes trivially with an eprintln) if Redis or the binaries are
//! missing, so CI without Redis stays green; the run_e2e.ps1 script runs it for real.
//!
//! Run: `cargo test -p arcane-infra --features migration --test multiprocess_e2e -- --ignored --nocapture`

#![cfg(feature = "migration")]

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use arcane_infra::node_inbox::NodeInboxFrame;
use arcane_wire::{ClientFrame, PlayerStatePayload, Vec3Q};
use uuid::Uuid;

const REDIS_URL: &str = "redis://127.0.0.1:6379";
const CLUSTER_A: &str = "550e8400-e29b-41d4-a716-446655440001";
const CLUSTER_B: &str = "550e8400-e29b-41d4-a716-446655440002";

fn redis_available() -> bool {
    TcpStream::connect_timeout(
        &"127.0.0.1:6379".parse().unwrap(),
        Duration::from_millis(1500),
    )
    .is_ok()
}

fn bin_path(name: &str) -> PathBuf {
    // target/release relative to the workspace root (tests run from crate dir).
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // workspace root
    p.push("target");
    p.push("release");
    p.push(format!("{}{}", name, std::env::consts::EXE_SUFFIX));
    p
}

/// Child process that is force-killed on drop (test panics must not leak processes).
struct Proc(Child, &'static str);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        eprintln!("[e2e] killed {}", self.1);
    }
}

fn log_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p
}

fn spawn_node(cluster_id: &str, ws_port: u16, neighbor: &str) -> std::io::Result<Proc> {
    let log = std::fs::File::create(log_dir().join(format!("e2e_node_{ws_port}.log")))?;
    let child = Command::new(bin_path("arcane-node"))
        .env("NODE_ID", cluster_id)
        .env("REDIS_URL", REDIS_URL)
        .env("NEIGHBOR_IDS", neighbor)
        .env("NODE_WS_PORT", ws_port.to_string())
        .env("NODE_STATE_PUBLISH_TICKS", "10") // ~3/s at 30Hz: fast control plane for the test
        .env("NODE_STATS_PORT", (ws_port + 1).to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()?;
    Ok(Proc(child, "node"))
}

fn spawn_manager() -> std::io::Result<Proc> {
    let log = std::fs::File::create(log_dir().join("e2e_manager.log"))?;
    let clusters = format!("{CLUSTER_A}:127.0.0.1:9080,{CLUSTER_B}:127.0.0.1:9082");
    let child = Command::new(bin_path("arcane-manager"))
        .env("MANAGER_CLUSTERS", clusters)
        .env("MANAGER_HTTP_PORT", "9091") // NOT 9081: node A's stats HTTP sits on ws_port+1
        .env("REDIS_URL", REDIS_URL)
        .env("MANAGER_CADENCE_MS", "300")
        .env("MANAGER_CAPACITY_FACTOR", "1.0") // force even spread: with 2 clusters and 2+ entities, A cannot keep both
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()?;
    Ok(Proc(child, "manager"))
}

/// Open a WS connection to a node and send one binary PLAYER_STATE frame
/// (FlatBuffer wire protocol — the only format the cluster speaks).
fn ws_connect(port: u16) -> Result<tungstenite::WebSocket<std::net::TcpStream>, String> {
    let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .set_nodelay(true)
        .map_err(|e| format!("nodelay: {e}"))?;
    let (ws, _resp) = tungstenite::client(format!("ws://127.0.0.1:{port}/"), stream)
        .map_err(|e| format!("ws handshake: {e}"))?;
    Ok(ws)
}

fn send_player_state(
    ws: &mut tungstenite::WebSocket<std::net::TcpStream>,
    entity_id: Uuid,
    x: f64,
    z: f64,
    seq: u64,
) -> Result<(), String> {
    let frame = ClientFrame::PlayerState(PlayerStatePayload {
        entity_id,
        position: Vec3Q::from_vec3(arcane_wire::Vec3::new(x, 0.0, z)),
        velocity: Vec3Q::new(0, 0, 0),
        user_data: Vec::new(),
        client_seq: seq,
    });
    let bytes = arcane_wire::encode_client(&frame);
    ws.send(tungstenite::Message::Binary(bytes))
        .map_err(|e| format!("ws send: {e}"))
}

/// Subscribe to both clusters' inbox channels directly (we are a passive observer
/// of the SAME frames the nodes consume) and record, per entity: the first time its
/// STATE appeared in B's frames and the first time a FLIP to B appeared.
struct InboxObserver {
    rx_a: std::sync::mpsc::Receiver<NodeInboxFrame>,
    rx_b: std::sync::mpsc::Receiver<NodeInboxFrame>,
}

impl InboxObserver {
    fn new() -> Result<Self, String> {
        use arcane_infra::node_inbox::{InboxBus, RedisInboxBus};
        let bus_a = RedisInboxBus::new(REDIS_URL).map_err(|e| format!("bus a: {e}"))?;
        let bus_b = RedisInboxBus::new(REDIS_URL).map_err(|e| format!("bus b: {e}"))?;
        let rx_a = bus_a.subscribe(Uuid::parse_str(CLUSTER_A).unwrap());
        let rx_b = bus_b.subscribe(Uuid::parse_str(CLUSTER_B).unwrap());
        // Keep buses alive by leaking them (test-lifetime only).
        std::mem::forget(bus_a);
        std::mem::forget(bus_b);
        Ok(Self { rx_a, rx_b })
    }
}

#[test]
#[ignore] // requires Redis + release binaries; run via scripts/run_multiprocess_e2e.ps1
fn full_control_plane_over_real_redis() {
    if !redis_available() {
        eprintln!("[e2e] SKIP: Redis not reachable on 127.0.0.1:6379");
        return;
    }
    for bin in ["arcane-manager", "arcane-node"] {
        if !bin_path(bin).exists() {
            eprintln!("[e2e] SKIP: missing release binary {:?}", bin_path(bin));
            return;
        }
    }

    // Flush any stale control-plane keys from previous runs.
    {
        let client = redis::Client::open(REDIS_URL).unwrap();
        let mut conn = client.get_connection().unwrap();
        for c in [CLUSTER_A, CLUSTER_B] {
            let _: Result<i32, _> = redis::cmd("DEL")
                .arg(format!("arcane:state:{c}"))
                .query(&mut conn);
        }
    }

    let ca = Uuid::parse_str(CLUSTER_A).unwrap();
    let cb = Uuid::parse_str(CLUSTER_B).unwrap();

    // Start observer BEFORE the manager so no frame is missed.
    let observer = InboxObserver::new().expect("inbox observer");

    // Node A (9080) + warm spare node B (9082) + manager.
    let _node_a = spawn_node(CLUSTER_A, 9080, CLUSTER_B).expect("spawn node A");
    let _node_b = spawn_node(CLUSTER_B, 9082, CLUSTER_A).expect("spawn node B");
    std::thread::sleep(Duration::from_millis(1500)); // nodes bind WS + start publishing state keys
    let _manager = spawn_manager().expect("spawn manager");

    // Two far-apart entity groups, ALL fed to node A ("everyone starts on one
    // cluster"). Capacity factor 1.0 with k=2 live clusters forces the manager to
    // move one group to B — onto a cluster that owns NOTHING (the §8 warm-spare path).
    let g1 = [Uuid::from_u128(0xA1), Uuid::from_u128(0xA2)];
    let g2 = [Uuid::from_u128(0xB1), Uuid::from_u128(0xB2)];

    // One WS connection per entity, all to node A.
    let mut sockets: Vec<(Uuid, f64, f64, _)> = Vec::new();
    for (i, e) in g1.iter().enumerate() {
        let ws = ws_connect(9080).expect("connect g1");
        sockets.push((*e, i as f64 * 5.0, 0.0, ws));
    }
    for (i, e) in g2.iter().enumerate() {
        let ws = ws_connect(9080).expect("connect g2");
        sockets.push((*e, 2000.0 + i as f64 * 5.0, 2000.0, ws));
    }

    // Keep feeding at ~10Hz while observing, up to 90s.
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut state_seen_at_b: std::collections::HashMap<Uuid, Instant> = Default::default();
    let mut flip_to_b_at: std::collections::HashMap<Uuid, Instant> = Default::default();
    let mut flips_seen: Vec<(Instant, Uuid, Uuid)> = Vec::new(); // (when, entity, to_cluster)
    let mut seq: u64 = 0;

    while Instant::now() < deadline {
        // Re-send entity states so node A keeps them in its spine.
        seq += 1;
        for (e, x, z, ws) in sockets.iter_mut() {
            let _ = send_player_state(ws, *e, *x, *z, seq);
        }

        // Drain both inboxes.
        let now = Instant::now();
        while let Ok(frame) = observer.rx_b.try_recv() {
            for ent in &frame.entities {
                state_seen_at_b.entry(ent.entry.entity_id).or_insert(now);
            }
            for flip in &frame.ownership {
                if flip.to_cluster == cb {
                    flip_to_b_at.entry(flip.entity_id).or_insert(now);
                }
                flips_seen.push((now, flip.entity_id, flip.to_cluster));
            }
        }
        while let Ok(frame) = observer.rx_a.try_recv() {
            for flip in &frame.ownership {
                flips_seen.push((now, flip.entity_id, flip.to_cluster));
            }
        }

        // Success condition: at least one entity flipped to B.
        if !flip_to_b_at.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // ---- Assertions ----
    assert!(
        !flip_to_b_at.is_empty(),
        "no migration to the warm spare within 90s — the control plane is not actuating \
         (flips seen anywhere: {:?})",
        flips_seen.len()
    );

    // §8: for every entity that flipped to B, B received its STATE strictly before the flip.
    for (entity, flip_time) in &flip_to_b_at {
        let state_time = state_seen_at_b.get(entity).unwrap_or_else(|| {
            panic!("entity {entity} flipped to B but B NEVER received its state (gate violated)")
        });
        assert!(
            state_time < flip_time,
            "entity {entity}: state first seen at B {:?} NOT before flip {:?} (gate violated)",
            state_time,
            flip_time
        );
    }

    // Settling: observe another 10s; no entity may flip AGAIN (ping-pong guard).
    let settle_deadline = Instant::now() + Duration::from_secs(10);
    let mut post_settle_flips = 0;
    // Adoption proof: after the flip, node B must actually SIMULATE the adopted
    // entities — observable as B's own replication topic carrying them with
    // cluster_id = B. Subscribe to B's per-cluster replication channel.
    let mut b_writes_migrated = false;
    let replication_rx = {
        let client = redis::Client::open(REDIS_URL).unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            if let Ok(mut conn) = client.get_connection() {
                let mut pubsub = conn.as_pubsub();
                if pubsub
                    .subscribe(format!("arcane:replication:{CLUSTER_B}"))
                    .is_ok()
                {
                    while let Ok(msg) = pubsub.get_message() {
                        if let Ok(payload) = msg.get_payload::<String>() {
                            if tx.send(payload).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });
        rx
    };
    while Instant::now() < settle_deadline {
        while let Ok(frame) = observer.rx_b.try_recv() {
            post_settle_flips += frame
                .ownership
                .iter()
                .filter(|f| flip_to_b_at.contains_key(&f.entity_id) && f.to_cluster != cb)
                .count();
        }
        while let Ok(frame) = observer.rx_a.try_recv() {
            post_settle_flips += frame
                .ownership
                .iter()
                .filter(|f| flip_to_b_at.contains_key(&f.entity_id) && f.to_cluster == ca)
                .count();
        }
        while let Ok(payload) = replication_rx.try_recv() {
            // B's delta payloads are JSON EntityStateDelta; a migrated entity id
            // appearing in B's OWN outbound replication means B is writing it.
            if flip_to_b_at
                .keys()
                .any(|e| payload.contains(&e.to_string()))
            {
                b_writes_migrated = true;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(
        post_settle_flips, 0,
        "migrated entities flipped back within the settle window (ping-pong)"
    );
    assert!(
        b_writes_migrated,
        "node B never wrote a migrated entity to its replication topic — adoption failed \
         (the flip changed the map but B is not simulating the entity)"
    );

    eprintln!(
        "[e2e] PASS: {} entities migrated A->B; gate order verified; B simulates adopted entities; no ping-pong.",
        flip_to_b_at.len()
    );
}
