//! Headless control-plane acceptance test: scripted players + per-cluster wire
//! observers, NO game client and NO graphics.
//!
//! Phases:
//! - `--phase static` (default): N static players join round-robin; verdict
//!   checks attribution, observer consistency, no flips (pinned), position
//!   continuity, and update cadence.
//! - `--phase migrate`: players 0 and 1 walk to a common point and STAY
//!   CONNECTED (default since D1). Run against the UNPINNED stack
//!   (`hl_stack.bat nopin`): the manager migrates the connected pair to
//!   co-locate it, and the D1 forwarding invariant keeps them correct — the
//!   old owner relays their inputs to the new owner instead of applying them
//!   (before D1 this scenario reproducibly FAILED: the flip split the entity,
//!   old owner fed by the client's WS echo vs new owner simulating the
//!   adopted copy, observers permanently disagreeing).
//!   `--disconnect-at-target` restores the legacy workaround behavior
//!   (players disconnect on arrival so the pin liveness window expires and
//!   entities migrate as plain server-side state — for the PINNED stack).
//!   Verdict additionally checks: the pair ends on the SAME cluster, at least
//!   one attribution flip occurred, and every flip was position-continuous.
//!
//! Everything is measured from the CLIENT WIRE — the same broadcast bytes a
//! game client renders from — so a pass here means a client on any cluster
//! would show correct per-cluster colors, including a live color change on
//! migration.
//!
//! Usage:
//!   migration_observer --manager http://127.0.0.1:7777 \
//!     --clusters ws://127.0.0.1:8080,ws://127.0.0.1:8082,... \
//!     --players 4 --duration 60 [--phase static|migrate]

use std::collections::HashMap;
use std::io::{Read as IoRead, Write as IoWrite};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Static,
    Migrate,
    /// #289 restart-convergence probe: static players; the RUNNER kills and
    /// restarts a node mid-run (scripts/hl_restart_node2.bat). Verdict checks
    /// only the FINAL WINDOW: after the disturbance settles, observers agree,
    /// nothing flips, every player still updates. Transient flips/disagreement
    /// during the disturbance are expected and allowed.
    Restart,
    /// Clustering-logic acceptance: 6 players walk into TWO tight groups
    /// (0,1,2 -> G1; 3,4,5 -> G2) far apart. The distance->interaction
    /// heuristic must produce the PREDICTED PARTITION: each group co-located
    /// on ONE cluster, the two groups on DIFFERENT clusters. Stronger than
    /// pair co-location: the full expected partition is asserted.
    Cluster,
    /// Clustering follow-through: same two groups; after a settle period
    /// player 0 DEFECTS and walks from G1 to G2. Verdict: the defector ends
    /// on G2's cluster (it followed its new neighbors) while both groups
    /// remain internally co-located.
    Defector,
    /// Attention-spectrum probe, ZERO end. Attention is ONE spectrum: how
    /// often a cluster hears about an entity, as a function of interaction
    /// probability p (the rate law; today quantized Zero/Low/Full, Zero =
    /// absent). This phase probes the zero-truncated end: 8 players in FOUR
    /// far corner groups, nothing approaching anything, p ~ 0 across groups.
    /// Run against the NOLEGACY stack. Verdict: each cluster broadcasts only
    /// its residents (the scaling payoff of the spectrum's floor). The
    /// `spectrum-warmup` phase probes the RISING part of the SAME curve — they are
    /// one mechanism, not two features. A/B: run without nolegacy for the
    /// world-broadcast baseline.
    SpectrumIdle,
    /// Attention-spectrum probe, RISING end — same mechanism as `spectrum-idle`,
    /// different region of the same p -> rate curve. Group A parked at
    /// (500,500), group B at (3000,500), a lone far control at (3000,8000).
    /// After 30s one B member (the TRAVELER, player 4) walks toward A at
    /// 50u/s: its p against A rises with approach, crossing from Zero
    /// (absent) into the included tiers BEFORE contact. Verdict, on A's
    /// host-cluster broadcast: the traveler appears before arrival (first
    /// post-settle sighting > 60u out, expect ~150-200u = screen radius
    /// minus pipeline latency) attributed to its FOREIGN owner (warmed as a
    /// proxy ahead of adoption), while the far control (p ~ 0) stays
    /// invisible throughout. Attention is likelihood-prioritized: the
    /// spectrum decides, distance only through p.
    SpectrumWarmup,
    /// Distance gradient: three PAIRS parked at increasing separation —
    /// close (~30u, inside proximity radius), mid (~400u), far (~4000u).
    /// Verdict: the close pair co-locates; the far pair NEVER does. Directly
    /// confirms distance <-> interaction probability. (The mid pair is
    /// reported but not asserted — it sits in the predictor's gray zone.)
    Gradient,
}

#[derive(Clone)]
struct Args {
    manager: String,
    clusters: Vec<String>,
    players: u32,
    duration_secs: u64,
    max_jump: f64,
    max_gap_ms: u64,
    phase: Phase,
    /// Legacy workaround mode for the PINNED stack: converging players
    /// disconnect on arrival so their entities migrate as server-side state.
    /// Default false since D1 (forwarding keeps CONNECTED players correct).
    disconnect_at_target: bool,
}

fn parse_args() -> Args {
    let mut manager = "http://127.0.0.1:7777".to_string();
    let mut clusters = vec![];
    let mut players = 4u32;
    let mut duration_secs = 60u64;
    let mut max_jump = 500.0f64;
    let mut max_gap_ms = 2000u64;
    let mut phase = Phase::Static;
    let mut disconnect_at_target = false;

    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--manager" => {
                manager = argv[i + 1].clone();
                i += 2;
            }
            "--clusters" => {
                clusters = argv[i + 1]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                i += 2;
            }
            "--players" => {
                players = argv[i + 1].parse().expect("--players");
                i += 2;
            }
            "--duration" => {
                duration_secs = argv[i + 1].parse().expect("--duration");
                i += 2;
            }
            "--max-jump" => {
                max_jump = argv[i + 1].parse().expect("--max-jump");
                i += 2;
            }
            "--max-gap-ms" => {
                max_gap_ms = argv[i + 1].parse().expect("--max-gap-ms");
                i += 2;
            }
            "--phase" => {
                phase = match argv[i + 1].as_str() {
                    "static" => Phase::Static,
                    "migrate" => Phase::Migrate,
                    "restart" => Phase::Restart,
                    "cluster" => Phase::Cluster,
                    "defector" => Phase::Defector,
                    "gradient" => Phase::Gradient,
                    "spectrum-idle" => Phase::SpectrumIdle,
                    "spectrum-warmup" => Phase::SpectrumWarmup,
                    other => {
                        eprintln!("unknown phase: {other}");
                        std::process::exit(2);
                    }
                };
                i += 2;
            }
            "--disconnect-at-target" => {
                disconnect_at_target = true;
                i += 1;
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }
    assert!(!clusters.is_empty(), "--clusters is required");
    if phase == Phase::Migrate {
        assert!(players >= 2, "--phase migrate needs at least 2 players");
    }
    Args {
        manager,
        clusters,
        players,
        duration_secs,
        max_jump,
        max_gap_ms,
        phase,
        disconnect_at_target,
    }
}

/// Minimal HTTP GET for localhost endpoints (avoids an HTTP client dependency).
fn http_get(url: &str) -> Result<String, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("only http:// supported: {url}"))?;
    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let mut stream = std::net::TcpStream::connect(host_port).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).map_err(|e| e.to_string())?;
    let body_start = buf
        .find("\r\n\r\n")
        .ok_or_else(|| "malformed HTTP response".to_string())?;
    Ok(buf[body_start + 4..].to_string())
}

#[derive(serde::Deserialize)]
struct JoinResponse {
    cluster_id: String,
    server_host: String,
    server_port: u16,
}

/// One observation of an entity by one observer.
#[derive(Clone)]
struct Observation {
    cluster_id: Uuid,
    position: (f64, f64, f64),
    at: Instant,
}

#[derive(Default)]
struct EntityTrack {
    /// Latest observation per observer index.
    latest: HashMap<usize, Observation>,
    /// (observer, from, to, position_delta, when) attribution changes.
    flips: Vec<(usize, Uuid, Uuid, f64, Instant)>,
    /// First sighting per observer AFTER the settle cutoff (25s): when, at
    /// what position, attributed to which cluster. The anticipation metric:
    /// distance-at-first-late-sighting on the destination's host observer.
    first_seen_late: HashMap<usize, (Instant, (f64, f64, f64), Uuid)>,
    /// Full sighting timeline per observer (when, position, attributed
    /// cluster) — the cadence record for the spectrum verdicts. Attribution
    /// distinguishes PROXY sightings (foreign-owned, spectrum-gated) from
    /// OWNED sightings (resident, sim-rate broadcast).
    sightings: HashMap<usize, Vec<(Instant, (f64, f64, f64), Uuid)>>,
    max_jump_seen: f64,
    max_gap_ms_seen: u128,
}

struct Shared {
    tracks: Mutex<HashMap<Uuid, EntityTrack>>,
    stop: AtomicBool,
    /// D2: RECONNECT frames followed by players (make-before-break moves).
    reconnects_followed: std::sync::atomic::AtomicU64,
    /// Program start; observers use it for the post-settle window.
    start: Instant,
    /// Router-inbox delivery events: (secs since start, consumer cluster,
    /// entity, rate_hz). The DIRECT record of the attention spectrum — each
    /// event is one router delivery of a foreign entity to a cluster, with
    /// the rate the spectrum assigned. (The WS wire can't measure this:
    /// nodes rebroadcast stale proxies on their own resync rhythm.)
    inbox_events: Mutex<Vec<(f64, Uuid, Uuid, f64)>>,
}

fn dist(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    let dz = a.2 - b.2;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn observer_thread(idx: usize, ws_url: String, shared: Arc<Shared>) {
    // Reconnect loop: a killed/restarted node (restart phase) must not
    // permanently blind its observer — a frozen observer would hold a stale
    // last-observation and fake a disagreement forever.
    'outer: while !shared.stop.load(Ordering::Relaxed) {
        let (mut socket, _) = match tungstenite::connect(&ws_url) {
            Ok(s) => s,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(1000));
                continue 'outer;
            }
        };
        eprintln!("[obs {idx}] connected to {ws_url}");
        while !shared.stop.load(Ordering::Relaxed) {
            let msg = match socket.read() {
                Ok(m) => m,
                Err(_) => {
                    eprintln!("[obs {idx}] connection lost; reconnecting");
                    std::thread::sleep(Duration::from_millis(1000));
                    continue 'outer;
                }
            };
        let tungstenite::Message::Binary(bytes) = msg else {
            continue;
        };
        let Ok(arcane_wire::ServerFrame::Delta(delta)) = arcane_wire::decode_server(&bytes) else {
            continue;
        };
        let now = Instant::now();
        let mut tracks = shared.tracks.lock().unwrap();
        for e in &delta.updated {
            let pos = e.position.to_vec3();
            let obs = Observation {
                cluster_id: e.cluster_id,
                position: (pos.x, pos.y, pos.z),
                at: now,
            };
            let track = tracks.entry(e.entity_id).or_default();
            if let Some(prev) = track.latest.get(&idx) {
                let gap = now.duration_since(prev.at).as_millis();
                // Re-acquisition: under interest-scoped replication an
                // observer legitimately loses sight of entities outside its
                // cluster's interest. After a long blind window the entity
                // may have moved arbitrarily far and changed owner
                // arbitrarily often — jump/flip accounting only makes sense
                // over CONTINUOUS observation. 5s >> the ~1s resync cadence,
                // so genuinely streamed entities never trip this.
                const REACQUIRE_MS: u128 = 5000;
                if gap <= REACQUIRE_MS {
                    let jump = dist(prev.position, obs.position);
                    if prev.cluster_id != obs.cluster_id {
                        track
                            .flips
                            .push((idx, prev.cluster_id, obs.cluster_id, jump, now));
                    } else if jump > track.max_jump_seen {
                        track.max_jump_seen = jump;
                    }
                    if gap > track.max_gap_ms_seen {
                        track.max_gap_ms_seen = gap;
                    }
                }
            }
                const SETTLE_CUTOFF_SECS: u64 = 25;
                if now.duration_since(shared.start).as_secs() >= SETTLE_CUTOFF_SECS {
                    track
                        .first_seen_late
                        .entry(idx)
                        .or_insert((now, obs.position, obs.cluster_id));
                }
                track
                    .sightings
                    .entry(idx)
                    .or_default()
                    .push((now, obs.position, obs.cluster_id));
                track.latest.insert(idx, obs);
            }
        }
    }
    eprintln!("[obs {idx}] done");
}

struct PlayerSpec {
    idx: u32,
    /// Restart phase: if the socket dies (its node was killed), re-join via
    /// the manager and keep driving the SAME entity — real clients reconnect.
    rejoin_on_failure: bool,
    spawn: (f64, f64),
    /// Walk toward this point at `speed` server-units/sec (None = static).
    target: Option<(f64, f64)>,
    /// Choreography: waypoints as (activate_after_secs, x, z). The player
    /// heads to the LAST waypoint whose activation time has elapsed
    /// (overrides `target` once any is active). Enables predicted-partition
    /// scenarios: park in a group, then move on schedule.
    waypoints: Vec<(f64, f64, f64)>,
    speed: f64,
    /// Legacy pinned-stack mode: stop sending and close the socket on
    /// reaching the target so the entity becomes plain server-side state.
    /// Default (false) since D1: the player STAYS CONNECTED through the
    /// flip and the forwarding invariant keeps it single-writer correct.
    disconnect_at_target: bool,
}

fn spawn_player(spec: PlayerSpec, manager: &str, shared: Arc<Shared>) -> Option<Uuid> {
    let idx = spec.idx;
    let manager_url = manager.trim_end_matches('/').to_string();
    let body = match http_get(&format!("{}/join", manager.trim_end_matches('/'))) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[player {idx}] join failed: {e}");
            return None;
        }
    };
    let join: JoinResponse = match serde_json::from_str(&body) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[player {idx}] join parse failed: {e} (body: {body})");
            return None;
        }
    };
    let entity_id = Uuid::new_v4();
    eprintln!(
        "[player {idx}] {entity_id} joined cluster {} at ({}, 0, {}) via {}:{} target={:?}",
        join.cluster_id, spec.spawn.0, spec.spawn.1, join.server_host, join.server_port, spec.target
    );

    let ws_url = format!("ws://{}:{}", join.server_host, join.server_port);
    std::thread::spawn(move || {
        let (mut socket, _) = match tungstenite::connect(&ws_url) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[player {idx}] ws connect failed: {e}");
                return;
            }
        };
        // Non-blocking reads so we can drain server broadcasts (never rendered,
        // but an unread socket would eventually stall the server's sender).
        if let tungstenite::stream::MaybeTlsStream::Plain(tcp) = socket.get_mut() {
            let _ = tcp.set_nonblocking(true);
        }
        let (mut x, mut z) = spec.spawn;
        let mut seq = 0u64;
        let dt = 0.1f64;
        let started = Instant::now();
        while !shared.stop.load(Ordering::Relaxed) {
            // Movement toward the target, if any. Waypoints override target:
            // head to the LAST waypoint whose activation time has elapsed.
            let elapsed = started.elapsed().as_secs_f64();
            let active_target: Option<(f64, f64)> = spec
                .waypoints
                .iter()
                .filter(|(after, _, _)| elapsed >= *after)
                .next_back()
                .map(|(_, wx, wz)| (*wx, *wz))
                .or(spec.target);
            let (mut vx, mut vz) = (0.0, 0.0);
            let mut arrived = false;
            if let Some((tx, tz)) = active_target {
                let dx = tx - x;
                let dz = tz - z;
                let d = (dx * dx + dz * dz).sqrt();
                let step = spec.speed * dt;
                if d > step {
                    vx = dx / d * spec.speed;
                    vz = dz / d * spec.speed;
                    x += dx / d * step;
                    z += dz / d * step;
                } else {
                    x = tx;
                    z = tz;
                    arrived = true;
                }
            }

            seq += 1;
            let frame = arcane_wire::ClientFrame::PlayerState(arcane_wire::PlayerStatePayload {
                entity_id,
                position: arcane_wire::Vec3Q::from_vec3(arcane_wire::Vec3::new(x, 0.0, z)),
                velocity: arcane_wire::Vec3Q::from_vec3(arcane_wire::Vec3::new(vx, 0.0, vz)),
                user_data: Vec::new(),
                client_seq: seq,
            });
            let bytes = arcane_wire::encode_client(&frame);
            if socket.send(tungstenite::Message::Binary(bytes)).is_err() {
                if spec.rejoin_on_failure {
                    // Our node died (restart phase). Re-join via the manager
                    // and keep driving the same entity id.
                    eprintln!("[player {idx}] send failed; re-joining via manager");
                    std::thread::sleep(Duration::from_millis(1500));
                    let Ok(body) = http_get(&format!("{manager_url}/join")) else {
                        continue;
                    };
                    let Ok(join) = serde_json::from_str::<JoinResponse>(&body) else {
                        continue;
                    };
                    let url = format!("ws://{}:{}", join.server_host, join.server_port);
                    match tungstenite::connect(&url) {
                        Ok((mut s, _)) => {
                            if let tungstenite::stream::MaybeTlsStream::Plain(tcp) = s.get_mut() {
                                let _ = tcp.set_nonblocking(true);
                            }
                            socket = s;
                            eprintln!("[player {idx}] re-joined via {url}");
                        }
                        Err(_) => continue,
                    }
                    continue;
                }
                eprintln!("[player {idx}] send failed, stopping");
                break;
            }
            if arrived && spec.disconnect_at_target {
                eprintln!("[player {idx}] arrived at target — disconnecting (entity persists server-side)");
                let _ = socket.close(None);
                break;
            }
            // Drain any buffered inbound frames (non-blocking). D2: a
            // RECONNECT frame for OUR entity triggers a make-before-break
            // move — connect to the new address, switch sends to it, then
            // drop the old socket. Timing is uncritical (D1 forwarding
            // keeps us correct while we switch), which is exactly what
            // this code demonstrates by doing it lazily mid-loop.
            let mut pending_redirect: Option<String> = None;
            loop {
                match socket.read() {
                    Ok(tungstenite::Message::Binary(bytes)) => {
                        if let Ok(arcane_wire::ServerFrame::Reconnect(rc)) =
                            arcane_wire::decode_server(&bytes)
                        {
                            if rc.entity_id == entity_id {
                                pending_redirect = Some(rc.addr.clone());
                            }
                        }
                    }
                    Ok(_) => continue,
                    Err(tungstenite::Error::Io(ref e))
                        if e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        break;
                    }
                    Err(_) => break,
                }
            }
            if let Some(addr) = pending_redirect {
                match tungstenite::connect(&addr) {
                    Ok((mut new_socket, _)) => {
                        if let tungstenite::stream::MaybeTlsStream::Plain(tcp) =
                            new_socket.get_mut()
                        {
                            let _ = tcp.set_nonblocking(true);
                        }
                        // Make-before-break: new connection is live; close old.
                        let old = std::mem::replace(&mut socket, new_socket);
                        drop(old);
                        shared
                            .reconnects_followed
                            .fetch_add(1, Ordering::Relaxed);
                        eprintln!("[player {idx}] followed RECONNECT to {addr}");
                    }
                    Err(e) => {
                        eprintln!("[player {idx}] RECONNECT to {addr} failed: {e}; staying (forwarding keeps us correct)");
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    Some(entity_id)
}

/// Discover which cluster an observer's node hosts by querying its stats
/// endpoint (ws port + 1). Gives verdicts a ground-truth observer->cluster
/// map instead of guessing from attribution (under interest scoping, several
/// observers legitimately broadcast the same entity).
fn observer_cluster(ws_url: &str) -> Option<Uuid> {
    let rest = ws_url.strip_prefix("ws://")?;
    let (host, port) = rest.split_once(':')?;
    let port: u16 = port.trim_end_matches('/').parse().ok()?;
    let body = http_get(&format!("http://{}:{}/stats", host, port + 1)).ok()?;
    let key = "\"cluster_id\":\"";
    let start = body.find(key)? + key.len();
    let end = body[start..].find('"')? + start;
    Uuid::parse_str(&body[start..end]).ok()
}

fn main() {
    let args = parse_args();
    let shared = Arc::new(Shared {
        tracks: Mutex::new(HashMap::new()),
        stop: AtomicBool::new(false),
        reconnects_followed: std::sync::atomic::AtomicU64::new(0),
        start: Instant::now(),
        inbox_events: Mutex::new(Vec::new()),
    });

    // Ground-truth observer->cluster identity (from each node's /stats).
    let obs_clusters: Vec<Option<Uuid>> = args
        .clusters
        .iter()
        .map(|ws| observer_cluster(ws))
        .collect();
    eprintln!("observer clusters: {obs_clusters:?}");

    // Router-inbox observers (spectrum phases): passively subscribe to each
    // cluster's inbox — the same frames the node consumes — and record every
    // foreign-entity delivery with its rate_hz. Redis pub/sub duplicates to
    // extra subscribers; the node is unaffected.
    if matches!(args.phase, Phase::SpectrumIdle | Phase::SpectrumWarmup) {
        use arcane_infra::node_inbox::{InboxBus, RedisInboxBus};
        for cluster in obs_clusters.iter().flatten().copied() {
            let shared_c = shared.clone();
            std::thread::spawn(move || {
                let Ok(bus) = RedisInboxBus::new("redis://127.0.0.1:6379") else {
                    return;
                };
                let rx = bus.subscribe(cluster);
                std::mem::forget(bus);
                while let Ok(frame) = rx.recv() {
                    if shared_c.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let now_s = Instant::now()
                        .duration_since(shared_c.start)
                        .as_secs_f64();
                    let mut ev = shared_c.inbox_events.lock().unwrap();
                    for re in &frame.entities {
                        ev.push((now_s, cluster, re.entry.entity_id, re.rate_hz));
                    }
                }
            });
        }
    }

    // Observers first (so they see players' first frames).
    for (idx, ws) in args.clusters.iter().enumerate() {
        let shared_c = shared.clone();
        let ws_c = ws.clone();
        std::thread::spawn(move || observer_thread(idx, ws_c, shared_c));
    }
    std::thread::sleep(Duration::from_secs(1));

    // Players. In migrate phase, players 0 and 1 spawn far apart and walk to a
    // common convergence point; the rest are static and far from everything.
    let convergence = (1500.0, 1500.0);
    let mut player_entities = vec![];
    for i in 0..args.players {
        // Cluster/Defector geometry: G1 around (500,500), G2 around (5500,5500)
        // — inter-group distance ~7000u >> proximity radius, intra-group ~30u.
        let group_slot = |center: (f64, f64), slot: u32| -> (f64, f64) {
            let offsets = [(0.0, 0.0), (30.0, 0.0), (0.0, 30.0)];
            let (ox, oz) = offsets[(slot % 3) as usize];
            (center.0 + ox, center.1 + oz)
        };
        let g1 = (500.0, 500.0);
        let g2 = (5500.0, 5500.0);

        let spec = match (args.phase, i) {
            (Phase::Migrate, 0) => PlayerSpec {
                idx: i,
                rejoin_on_failure: false,
                spawn: (100.0, 100.0),
                target: Some(convergence),
                waypoints: vec![],
                speed: 60.0,
                disconnect_at_target: args.disconnect_at_target,
            },
            (Phase::Migrate, 1) => PlayerSpec {
                idx: i,
                rejoin_on_failure: false,
                spawn: (3000.0, 3000.0),
                target: Some(convergence),
                waypoints: vec![],
                speed: 60.0,
                disconnect_at_target: args.disconnect_at_target,
            },
            (Phase::Migrate, _) => PlayerSpec {
                // Static bystanders live in far corners, away from the
                // convergence point: no proximity edges, no partition
                // pressure, no reason to migrate.
                idx: i,
                rejoin_on_failure: false,
                spawn: (5000.0 + 800.0 * i as f64, 200.0),
                target: None,
                waypoints: vec![],
                speed: 0.0,
                disconnect_at_target: false,
            },
            (Phase::Restart, _) => PlayerSpec {
                // Far-apart static players that SURVIVE their node dying:
                // re-join through the manager and keep driving the entity.
                idx: i,
                rejoin_on_failure: true,
                spawn: (100.0 + 1500.0 * i as f64, 100.0 + 1500.0 * i as f64),
                target: None,
                waypoints: vec![],
                speed: 0.0,
                disconnect_at_target: false,
            },
            (Phase::Cluster, _) => {
                // Players spawn SCATTERED (one per cluster via round-robin,
                // far apart), then walk into their group. The partition must
                // be DISCOVERED by the distance heuristic, not inherited
                // from the join placement.
                let group = if i < 3 { g1 } else { g2 };
                let slot = group_slot(group, i % 3);
                PlayerSpec {
                    idx: i,
                    rejoin_on_failure: false,
                    spawn: (100.0 + 2000.0 * i as f64, 8000.0),
                    target: Some(slot),
                    waypoints: vec![],
                    speed: 120.0,
                    disconnect_at_target: false,
                }
            }
            (Phase::Defector, _) => {
                let group = if i < 3 { g1 } else { g2 };
                let slot = group_slot(group, i % 3);
                let mut waypoints = vec![];
                if i == 0 {
                    // The defector: after the groups settle (45s), walk to G2.
                    waypoints.push((45.0, g2.0 - 30.0, g2.1 - 30.0));
                }
                PlayerSpec {
                    idx: i,
                    rejoin_on_failure: false,
                    spawn: (100.0 + 2000.0 * i as f64, 8000.0),
                    target: Some(slot),
                    waypoints,
                    speed: 120.0,
                    disconnect_at_target: false,
                }
            }
            (Phase::SpectrumIdle, _) => {
                // Four tight groups in four far corners; 2 players per group.
                // Groups are ~14000u apart — far beyond proximity and screen
                // radius, so cross-group interest must be ZERO.
                let corners = [
                    (500.0, 500.0),
                    (10500.0, 500.0),
                    (500.0, 10500.0),
                    (10500.0, 10500.0),
                ];
                let group = (i / 2) as usize % 4;
                let slot = (i % 2) as f64;
                let (cx, cz) = corners[group];
                PlayerSpec {
                    idx: i,
                    rejoin_on_failure: false,
                    spawn: (5000.0 + 300.0 * i as f64, 5200.0),
                    target: Some((cx + slot * 30.0, cz)),
                    waypoints: vec![],
                    speed: 250.0,
                    disconnect_at_target: false,
                }
            }
            (Phase::SpectrumWarmup, _) => {
                // 0,1: group A. 2,3: group B (+ traveler 4 starts IN B so its
                // initial owner is B's cluster, guaranteed by proximity
                // edges). 5: lone far control. Traveler departs at 30s and
                // approaches A at 50u/s (2500u ≈ 50s travel): slow enough
                // that screen(200u) -> promote -> route -> broadcast happens
                // well before proximity-radius arrival (50u).
                // B parks 800u from A: beyond the 200u screen radius (p=0,
                // invisible) but a short walk. The traveler approaches at
                // 20u/s (~40s to contact) so p rises SLOWLY through the
                // screen window and the cadence gradient is observable; at
                // t=90s it retreats.
                let (spawn, waypoints): ((f64, f64), Vec<(f64, f64, f64)>) = match i {
                    0 => ((500.0, 500.0), vec![]),
                    1 => ((530.0, 500.0), vec![]),
                    2 => ((1300.0, 500.0), vec![]),
                    3 => ((1330.0, 500.0), vec![]),
                    4 => ((1300.0, 530.0), vec![(30.0, 530.0, 530.0), (90.0, 1300.0, 530.0)]),
                    _ => ((3000.0, 8000.0), vec![]),
                };
                PlayerSpec {
                    idx: i,
                    rejoin_on_failure: false,
                    spawn,
                    target: None,
                    waypoints,
                    speed: 20.0,
                    disconnect_at_target: false,
                }
            }
            (Phase::Gradient, _) => {
                // Three pairs at increasing separation. Pair k = players
                // (2k, 2k+1). Bases far apart so pairs never interact.
                let pair = i / 2;
                let side = (i % 2) as f64;
                let (bx, bz, sep) = match pair {
                    0 => (500.0, 500.0, 30.0),     // close: inside proximity radius
                    1 => (500.0, 6000.0, 400.0),   // mid: predictor gray zone
                    _ => (6000.0, 500.0, 4000.0),  // far: no interaction ever
                };
                PlayerSpec {
                    idx: i,
                    rejoin_on_failure: false,
                    spawn: (100.0 + 2000.0 * i as f64, 12000.0),
                    target: Some((bx + side * sep, bz)),
                    waypoints: vec![],
                    speed: 200.0,
                    disconnect_at_target: false,
                }
            }
            _ => PlayerSpec {
                idx: i,
                rejoin_on_failure: false,
                spawn: (100.0 + 700.0 * i as f64, 100.0 + 700.0 * i as f64),
                target: None,
                waypoints: vec![],
                speed: 0.0,
                disconnect_at_target: false,
            },
        };
        if let Some(id) = spawn_player(spec, &args.manager, shared.clone()) {
            player_entities.push(id);
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    assert_eq!(
        player_entities.len(),
        args.players as usize,
        "not all players joined"
    );

    // Run + periodic report.
    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(5));
        let tracks = shared.tracks.lock().unwrap();
        eprintln!("--- attribution ({} entities tracked) ---", tracks.len());
        for id in &player_entities {
            match tracks.get(id) {
                Some(t) => {
                    let mut owners: Vec<String> = t
                        .latest
                        .iter()
                        .map(|(obs, o)| format!("obs{}={}", obs, &o.cluster_id.to_string()[..8]))
                        .collect();
                    owners.sort();
                    eprintln!(
                        "  player {}: {} flips={} max_jump={:.1} max_gap={}ms",
                        &id.to_string()[..8],
                        owners.join(" "),
                        t.flips.len(),
                        t.max_jump_seen,
                        t.max_gap_ms_seen
                    );
                }
                None => eprintln!("  player {}: NOT OBSERVED YET", &id.to_string()[..8]),
            }
        }
    }
    shared.stop.store(true, Ordering::Relaxed);

    // ── Verdict ────────────────────────────────────────────────────────────
    let tracks = shared.tracks.lock().unwrap();
    let mut failures: Vec<String> = vec![];

    for (i, id) in player_entities.iter().enumerate() {
        let Some(t) = tracks.get(id) else {
            failures.push(format!("player {i} ({id}): never observed on any cluster"));
            continue;
        };
        // Consistency: every observer that sees the entity agrees on the owner.
        // In migrate phase the converging pair (0,1) legitimately transitions
        // owners; observers may be mid-propagation at the sampling instant, so
        // the disagree check applies only to entities that should be stable.
        let owners: std::collections::HashSet<Uuid> =
            t.latest.values().map(|o| o.cluster_id).collect();
        let transitioning = args.phase == Phase::Migrate && i < 2;
        // Restart/cluster/defector/gradient phases: agreement is only required
        // at the END (final-window pass below); entities legitimately
        // transition owners mid-run while the partition is being discovered.
        let end_state_phase = matches!(
            args.phase,
            Phase::Restart
                | Phase::Cluster
                | Phase::Defector
                | Phase::Gradient
                | Phase::SpectrumIdle
                | Phase::SpectrumWarmup
        );
        if owners.len() > 1 && !transitioning && !end_state_phase {
            failures.push(format!(
                "player {i} ({id}): observers DISAGREE on owner: {owners:?}"
            ));
        }
        // Flips: forbidden only in the static phase; every other phase
        // exercises migration by design (the end-state checks catch
        // non-settling).
        let flips_allowed = args.phase != Phase::Static;
        if !flips_allowed && !t.flips.is_empty() {
            failures.push(format!(
                "player {i} ({id}): {} attribution flip(s) despite pinning",
                t.flips.len()
            ));
        }
        // Every flip must be position-continuous (the same entity may not
        // teleport when its owner changes — the §8 adoption seed guarantees it).
        for (obs, from, to, jump, _) in &t.flips {
            if *jump > args.max_jump {
                failures.push(format!(
                    "player {i} ({id}): flip {from}->{to} on obs{obs} jumped {jump:.1} > {:.1}",
                    args.max_jump
                ));
            }
        }
        if t.max_jump_seen > args.max_jump {
            failures.push(format!(
                "player {i} ({id}): position jump {:.1} > {:.1} (teleport class)",
                t.max_jump_seen, args.max_jump
            ));
        }
        if t.max_gap_ms_seen > args.max_gap_ms as u128
            && !matches!(
                args.phase,
                Phase::Restart
                    | Phase::Cluster
                    | Phase::Defector
                    | Phase::Gradient
                    | Phase::SpectrumIdle
                    | Phase::SpectrumWarmup
            )
        {
            // Restart phase: the kill window legitimately gaps; freshness is
            // asserted in the final-window pass instead.
            failures.push(format!(
                "player {i} ({id}): update gap {}ms > {}ms (stale/slow class)",
                t.max_gap_ms_seen, args.max_gap_ms
            ));
        }
    }

    // ── Clustering phases: PREDICTED-PARTITION verdicts ────────────────────
    // The majority-owner of each entity at the end, from the freshest
    // observations (same resolution the migrate phase uses).
    let end_owner = |id: &Uuid| -> Option<Uuid> {
        let t = tracks.get(id)?;
        let mut counts: HashMap<Uuid, usize> = HashMap::new();
        for o in t.latest.values() {
            *counts.entry(o.cluster_id).or_default() += 1;
        }
        counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c)
    };

    if matches!(args.phase, Phase::Cluster | Phase::Defector) {
        assert!(args.players >= 6, "cluster/defector phases need 6 players");
        // Expected partition: {0,1,2} together, {3,4,5} together, apart.
        // Defector: player 0 must END with group 2 instead.
        let (g1_idx, g2_idx): (Vec<usize>, Vec<usize>) = match args.phase {
            Phase::Defector => (vec![1, 2], vec![0, 3, 4, 5]),
            _ => (vec![0, 1, 2], vec![3, 4, 5]),
        };
        let owners_of = |idxs: &[usize]| -> Vec<Option<Uuid>> {
            idxs.iter()
                .map(|i| end_owner(&player_entities[*i]))
                .collect()
        };
        let g1_owners = owners_of(&g1_idx);
        let g2_owners = owners_of(&g2_idx);
        eprintln!("group1 (players {g1_idx:?}) owners: {g1_owners:?}");
        eprintln!("group2 (players {g2_idx:?}) owners: {g2_owners:?}");

        let g1_set: std::collections::HashSet<_> = g1_owners.iter().flatten().collect();
        let g2_set: std::collections::HashSet<_> = g2_owners.iter().flatten().collect();
        if g1_owners.iter().any(|o| o.is_none()) || g1_set.len() != 1 {
            failures.push(format!(
                "group 1 NOT co-located on one cluster: {g1_owners:?} — \
                 distance heuristic failed to cluster the group"
            ));
        }
        if g2_owners.iter().any(|o| o.is_none()) || g2_set.len() != 1 {
            failures.push(format!(
                "group 2 NOT co-located on one cluster: {g2_owners:?} — \
                 distance heuristic failed to cluster the group"
            ));
        }
        if g1_set.len() == 1 && g2_set.len() == 1 && g1_set == g2_set {
            failures.push(
                "BOTH groups on the SAME cluster — partition ignored inter-group distance"
                    .to_string(),
            );
        }
        if args.phase == Phase::Defector {
            let defector = end_owner(&player_entities[0]);
            let g2_owner = g2_owners.get(1).copied().flatten(); // player 3's owner
            eprintln!("defector (player 0) end owner: {defector:?} vs group2 {g2_owner:?}");
            if defector.is_none() || defector != g2_owner {
                failures.push(format!(
                    "defector did NOT follow its new group: ended {defector:?}, group2 on {g2_owner:?}"
                ));
            }
        }
    }

    if args.phase == Phase::Gradient {
        assert!(args.players >= 6, "gradient phase needs 6 players");
        let pair_owner = |k: usize| -> (Option<Uuid>, Option<Uuid>) {
            (
                end_owner(&player_entities[2 * k]),
                end_owner(&player_entities[2 * k + 1]),
            )
        };
        let (c0, c1) = pair_owner(0); // close pair
        let (m0, m1) = pair_owner(1); // mid pair (reported, not asserted)
        let (f0, f1) = pair_owner(2); // far pair
        eprintln!("close pair (30u):  {c0:?} vs {c1:?}");
        eprintln!("mid pair (400u):   {m0:?} vs {m1:?} (informational)");
        eprintln!("far pair (4000u):  {f0:?} vs {f1:?}");
        if c0.is_none() || c0 != c1 {
            failures.push(format!(
                "CLOSE pair (30u apart) not co-located: {c0:?} vs {c1:?} — \
                 high interaction probability ignored"
            ));
        }
        // The far pair must never co-locate... unless capacity packing put
        // them together for free (k=4 clusters, 6 entities — packing can
        // legally pair leftovers). What we assert is the CONTRAST: the far
        // pair must not be treated BETTER than the close pair. If the close
        // pair co-located and the far pair is on one cluster too, require
        // that no flip was needed to achieve it (i.e., they were simply
        // packed, not attracted): the far pair's tracks must show zero flips.
        if f0.is_some() && f0 == f1 {
            let far_flips: usize = [4usize, 5usize]
                .iter()
                .filter_map(|i| tracks.get(&player_entities[*i]))
                .map(|t| t.flips.len())
                .sum();
            if far_flips > 0 {
                failures.push(format!(
                    "FAR pair (4000u apart) was actively migrated together ({far_flips} flips) — \
                     distance should have kept interaction probability ~0"
                ));
            } else {
                eprintln!("far pair co-resident by initial packing (0 flips) — acceptable");
            }
        }
    }

    // ── Attention phase: interest-scoped visibility per observer ───────────
    if args.phase == Phase::SpectrumIdle {
        assert!(args.players >= 8, "spectrum-idle phase needs 8 players");
        let final_window = Duration::from_secs(10);
        let now = Instant::now();

        // fresh_vis[k] = set of PLAYER entities observer k saw in the window.
        let n_obs = args.clusters.len();
        let mut fresh_vis: Vec<std::collections::HashSet<Uuid>> =
            vec![Default::default(); n_obs];
        for id in &player_entities {
            if let Some(t) = tracks.get(id) {
                for (obs, o) in &t.latest {
                    if now.duration_since(o.at) < final_window {
                        fresh_vis[*obs].insert(*id);
                    }
                }
            }
        }

        // Group membership by construction: players 2k,2k+1 form group k.
        let group_of = |id: &Uuid| -> usize {
            player_entities.iter().position(|e| e == id).unwrap() / 2
        };
        // An observer HOSTS group g if any member's end-owner cluster is the
        // cluster this observer watches... we don't know observer->cluster
        // mapping directly, but each entity's fresh observation carries the
        // owner cluster; the HOSTING observer is the one that sees the group
        // with its own cluster_id. Simpler and stronger: for each observer,
        // the groups it sees freshly must be exactly the groups whose owner
        // cluster is the one it predominantly reports for those entities —
        // i.e. each observer sees its own residents (+ nothing far away).
        //
        // Practical check: every group must be freshly visible on AT LEAST
        // ONE observer (its host), and NO observer may freshly see ALL
        // groups (that would mean world-broadcast, no attention scoping) —
        // with 4 far groups on 4 clusters, each observer should carry ~its
        // own residents only.
        let mut per_obs_group_counts: Vec<Vec<usize>> = Vec::new();
        for vis in fresh_vis.iter() {
            let mut counts = vec![0usize; 4];
            for id in vis {
                counts[group_of(id)] += 1;
            }
            per_obs_group_counts.push(counts);
        }
        for (obs, counts) in per_obs_group_counts.iter().enumerate() {
            let total: usize = counts.iter().sum();
            eprintln!(
                "observer {obs}: fresh entities={total} groups={counts:?}"
            );
        }
        // Each group is hosted somewhere.
        for g in 0..4 {
            let hosted = per_obs_group_counts.iter().any(|c| c[g] == 2);
            if !hosted {
                failures.push(format!(
                    "group {g} not fully visible on ANY observer — its host cluster is not broadcasting it"
                ));
            }
        }
        // Attention scoping: no observer sees every group.
        let world_broadcasters = per_obs_group_counts
            .iter()
            .enumerate()
            .filter(|(_, c)| c.iter().all(|&n| n > 0))
            .count();
        if world_broadcasters > 0 {
            failures.push(format!(
                "{world_broadcasters} observer(s) freshly see ALL FOUR far-apart groups — \
                 replication is world-broadcast, not interest-scoped"
            ));
        }
        // Quantify: total fresh visibility across observers vs the
        // world-broadcast worst case (n_obs * players).
        let total_vis: usize = fresh_vis.iter().map(|v| v.len()).sum();
        let worst = n_obs * args.players as usize;
        eprintln!(
            "attention scaling: {total_vis} fresh entity-observations across {n_obs} clusters \
             vs {worst} under world-broadcast ({}% of worst case)",
            (100 * total_vis) / worst.max(1)
        );
    }

    // ── Spectrum-warmup verdict: the p -> rate curve, both directions ──────
    //
    // All identification is anchored to the APPROACH WINDOW (not the end
    // state): late repartitions after the retreat can legally reshuffle
    // hosts, so end-state identities are confounded. Capacity guarantees
    // A's settle-time owner differs from the traveler's (A=2 and B=3 cannot
    // share a cluster at capacity 3), so window-anchored checks are sound.
    if args.phase == Phase::SpectrumWarmup {
        assert!(args.players >= 6, "spectrum-warmup phase needs 6 players");
        let a_center = (500.0f64, 500.0f64);
        let depart_secs = 30.0f64;

        // Owner-at-time reconstruction: walk the flips (which
        // carry timestamps) per entity; owner(t) = last flip.to before t,
        // else the earliest flip.from, else the (single) attribution in
        // `latest`. Falls back gracefully when an entity never flipped.
        let owner_at = |id: &Uuid, at_secs: f64| -> Option<Uuid> {
            let t = tracks.get(id)?;
            let mut fl: Vec<(f64, Uuid, Uuid)> = t
                .flips
                .iter()
                .map(|(_, from, to, _, when)| {
                    (when.duration_since(shared.start).as_secs_f64(), *from, *to)
                })
                .collect();
            fl.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let mut owner: Option<Uuid> = None;
            for (s, from, to) in &fl {
                if *s <= at_secs {
                    owner = Some(*to);
                } else if owner.is_none() {
                    owner = Some(*from);
                }
            }
            owner.or_else(|| t.latest.values().next().map(|o| o.cluster_id))
        };

        let a_owner = owner_at(&player_entities[0], 28.0);
        let traveler = player_entities[4];
        let trav_owner_pre = owner_at(&traveler, 28.0);
        if a_owner.is_none() {
            failures.push("cannot identify A's settle-time owner".to_string());
        }
        if a_owner.is_some() && a_owner == trav_owner_pre {
            failures.push(format!(
                "scenario collision: A and the traveler share a settle-time owner \
                 ({a_owner:?}) — capacity should forbid this; partition suspect"
            ));
        }
        let host = a_owner.and_then(|own| obs_clusters.iter().position(|c| *c == Some(own)));
        match host {
            None => failures.push(format!(
                "no observer hosts A's settle-time cluster {a_owner:?} \
                 (observer map {obs_clusters:?})"
            )),
            Some(host) => {
                let dist_to_a = |p: &(f64, f64, f64)| -> f64 {
                    let dx = p.0 - a_center.0;
                    let dz = p.2 - a_center.1;
                    (dx * dx + dz * dz).sqrt()
                };
                let sights = tracks
                    .get(&traveler)
                    .and_then(|t| t.sightings.get(&host))
                    .cloned()
                    .unwrap_or_default();

                let host_cluster = a_owner.unwrap();
                // 1) Pre-departure: while the traveler is PARKED 800u away
                //    its p against A is at/near the spectrum floor, so A's
                //    host may deliver it at most RARELY. Assert on the rate
                //    of position-changed proxy deliveries in the parked
                //    window (< depart): sparse (< 1 per 5s) is the floor
                //    working; dense means attention leaked. Attribution
                //    alone can't be the test — A and B sit close enough for
                //    legitimate weak edges.
                let parked_window_secs = depart_secs - 2.0;
                let mut parked_deliveries = 0usize;
                let mut lastp: Option<(f64, f64, f64)> = None;
                for (when, pos, attributed) in &sights {
                    let s = when.duration_since(shared.start).as_secs_f64();
                    if s >= parked_window_secs {
                        break;
                    }
                    if *attributed == host_cluster {
                        continue;
                    }
                    let changed = lastp.is_none_or(|lp| {
                        let dx = lp.0 - pos.0;
                        let dz = lp.2 - pos.2;
                        (dx * dx + dz * dz).sqrt() > 0.5
                    });
                    lastp = Some(*pos);
                    if changed {
                        parked_deliveries += 1;
                    }
                }
                let parked_rate_per_5s =
                    parked_deliveries as f64 * 5.0 / parked_window_secs.max(1.0);
                eprintln!(
                    "PARKED: {parked_deliveries} proxy deliveries in {parked_window_secs:.0}s \
                     ({parked_rate_per_5s:.2} per 5s)"
                );
                if parked_rate_per_5s > 1.0 {
                    failures.push(format!(
                        "parked traveler delivered at {parked_rate_per_5s:.1}/5s on A's host — \
                         spectrum floor not suppressing a low-likelihood entity"
                    ));
                }

                // 2) Anticipation: first sighting on A's host mid-approach —
                //    before contact (>60u out) but after departure.
                let first_appr = sights.iter().find(|(when, _, _)| {
                    when.duration_since(shared.start).as_secs_f64() >= depart_secs - 1.0
                });
                match first_appr {
                    Some((when, pos, _)) => {
                        let d = dist_to_a(pos);
                        let s = when.duration_since(shared.start).as_secs_f64();
                        eprintln!(
                            "ANTICIPATION: A's host obs{host} first saw traveler at \
                             t={s:.1}s, {d:.0}u from A"
                        );
                        if d <= 60.0 {
                            failures.push(format!(
                                "traveler only became visible {d:.0}u from A — no \
                                 anticipatory replication (expected warm-up ~150-200u)"
                            ));
                        }
                    }
                    None => failures.push(
                        "A's host never saw the traveler during the approach — \
                         no anticipatory replication at all"
                            .to_string(),
                    ),
                }

                // 3) The spectrum, measured at the ROUTER INBOX — the direct
                //    record (each event = one router delivery of the traveler
                //    to A's host, with the assigned rate_hz). The WS wire
                //    cannot measure this: nodes rebroadcast stale proxies on
                //    their own resync rhythm. Distance derives from the walk
                //    schedule: parked 800u until depart, then closing 20u/s.
                //    Functional monotonicity only: nearer -> assigned rate not
                //    lower AND delivery gaps not longer.
                let dist_at = |s: f64| -> f64 {
                    if s < depart_secs {
                        800.0
                    } else {
                        (800.0 - 20.0 * (s - depart_secs)).max(30.0)
                    }
                };
                let events = shared.inbox_events.lock().unwrap();
                let mut far_ts: Vec<f64> = vec![];
                let mut near_ts: Vec<f64> = vec![];
                let mut far_rates: Vec<f64> = vec![];
                let mut near_rates: Vec<f64> = vec![];
                for (s, cluster, entity, rate) in events.iter() {
                    if *cluster != host_cluster || *entity != traveler {
                        continue;
                    }
                    let d = dist_at(*s);
                    if (300.0..700.0).contains(&d) {
                        far_ts.push(*s);
                        far_rates.push(*rate);
                    } else if (60.0..250.0).contains(&d) {
                        near_ts.push(*s);
                        near_rates.push(*rate);
                    }
                }
                drop(events);
                let median = |v: &mut Vec<f64>| -> Option<f64> {
                    if v.is_empty() {
                        return None;
                    }
                    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    Some(v[v.len() / 2])
                };
                let mut far_gaps: Vec<f64> =
                    far_ts.windows(2).map(|w| w[1] - w[0]).collect();
                let mut near_gaps: Vec<f64> =
                    near_ts.windows(2).map(|w| w[1] - w[0]).collect();
                match (
                    median(&mut far_gaps),
                    median(&mut near_gaps),
                    median(&mut far_rates),
                    median(&mut near_rates),
                ) {
                    (Some(fg), Some(ng), Some(fr), Some(nr)) => {
                        eprintln!(
                            "SPECTRUM (router inbox): far band 300-700u — {} deliveries, \
                             median gap {fg:.2}s, median rate {fr:.2}Hz; near band 60-250u — \
                             {} deliveries, median gap {ng:.2}s, median rate {nr:.2}Hz",
                            far_ts.len(),
                            near_ts.len()
                        );
                        if nr < fr {
                            failures.push(format!(
                                "assigned rate did NOT rise with proximity: near \
                                 {nr:.2}Hz < far {fr:.2}Hz"
                            ));
                        }
                        if ng > fg * 1.5 {
                            failures.push(format!(
                                "delivery cadence did NOT increase with proximity: \
                                 near gap {ng:.2}s vs far gap {fg:.2}s"
                            ));
                        }
                    }
                    _ => {
                        eprintln!(
                            "SPECTRUM: far {} / near {} inbox deliveries",
                            far_ts.len(),
                            near_ts.len()
                        );
                        if near_ts.is_empty() {
                            failures.push(
                                "no router deliveries in the near band — spectrum not \
                                 delivering as p rises"
                                    .to_string(),
                            );
                        }
                    }
                }

                // 4) The far control (player 5): p ~ 0 all run — never a
                //    PROXY on A's host. (It may legitimately be OWNED by the
                //    same cluster via capacity packing; owned broadcast is
                //    residency, not attention.)
                let far_ctl = player_entities[5];
                // Exemption: the manager may LEGALLY migrate the far control
                // onto/off A's host (capacity packing of an edge-less
                // leftover is an ownership decision, not an attention one).
                // Warm-up/hand-off proxies within +-20s of such a flip are
                // expected; proxy sightings OUTSIDE those windows are the
                // actual attention leak.
                let far_flip_times: Vec<f64> = tracks
                    .get(&far_ctl)
                    .map(|t| {
                        t.flips
                            .iter()
                            .filter(|(_, from, to, _, _)| {
                                *from == host_cluster || *to == host_cluster
                            })
                            .map(|(_, _, _, _, when)| {
                                when.duration_since(shared.start).as_secs_f64()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let far_proxy_leak = tracks
                    .get(&far_ctl)
                    .and_then(|t| t.sightings.get(&host))
                    .is_some_and(|s| {
                        s.iter().any(|(when, _, attr)| {
                            if *attr == host_cluster {
                                return false;
                            }
                            let ts = when.duration_since(shared.start).as_secs_f64();
                            !far_flip_times.iter().any(|ft| (ts - ft).abs() < 20.0)
                        })
                    });
                if far_proxy_leak {
                    failures.push(
                        "far control was proxied on A's host outside any migration \
                         window — attention leaked to a zero-likelihood entity"
                            .to_string(),
                    );
                }
            }
        }
    }

    // ── Restart phase: FINAL-WINDOW convergence (#289 acceptance) ──────────
    // After the disturbance settles, the system must be indistinguishable
    // from a healthy one: every player fresh, observers agree, no flips in
    // the final window. Un-fakeable: a node that failed to converge keeps
    // flipping attribution or freezes its entity, tripping these checks.
    if args.phase == Phase::Restart {
        let final_window = Duration::from_secs(15);
        let now = Instant::now();
        for (i, id) in player_entities.iter().enumerate() {
            let Some(t) = tracks.get(id) else { continue };
            // Freshness: latest observation within 3s across surviving observers.
            let freshest = t.latest.values().map(|o| o.at).max();
            match freshest {
                Some(at) if now.duration_since(at) < Duration::from_secs(3) => {}
                _ => failures.push(format!(
                    "player {i} ({id}): NOT FRESH at end (last obs {:?} ago) — frozen after restart",
                    freshest.map(|a| now.duration_since(a))
                )),
            }
            // End-state agreement across observers with a FRESH view (an
            // observer whose latest is older than the final window is mid-
            // reconnect backlog, not a divergent authority).
            let fresh_owners: std::collections::HashSet<Uuid> = t
                .latest
                .values()
                .filter(|o| now.duration_since(o.at) < final_window)
                .map(|o| o.cluster_id)
                .collect();
            if fresh_owners.len() > 1 {
                failures.push(format!(
                    "player {i} ({id}): observers DISAGREE at end: {fresh_owners:?}"
                ));
            }
            // Settled: no flips in the final window.
            let late_flips = t
                .flips
                .iter()
                .filter(|(_, _, _, _, at)| now.duration_since(*at) < final_window)
                .count();
            if late_flips > 0 {
                failures.push(format!(
                    "player {i} ({id}): {late_flips} flip(s) in the final {}s — not settled",
                    final_window.as_secs()
                ));
            }
        }
    }

    // Distribution: with round-robin joins, players should span >1 cluster.
    let distinct_owners: std::collections::HashSet<Uuid> = player_entities
        .iter()
        .filter_map(|id| tracks.get(id))
        .flat_map(|t| t.latest.values().map(|o| o.cluster_id))
        .collect();
    eprintln!(
        "distinct owner clusters across players: {}",
        distinct_owners.len()
    );
    if args.players >= 2 && distinct_owners.len() < 2 && args.phase == Phase::Static {
        failures.push(format!(
            "all {} players attributed to ONE cluster — round-robin/colors cannot be seen",
            args.players
        ));
    }

    // Migrate phase: the converging pair must end co-located, via >= 1 flip.
    if args.phase == Phase::Migrate {
        // Majority owner across observers (a lagging observer's stale copy
        // shouldn't fail a genuinely converged pair).
        let owner_of = |id: &Uuid| -> Option<Uuid> {
            let t = tracks.get(id)?;
            let mut counts: HashMap<Uuid, usize> = HashMap::new();
            for o in t.latest.values() {
                *counts.entry(o.cluster_id).or_default() += 1;
            }
            counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c)
        };
        match (owner_of(&player_entities[0]), owner_of(&player_entities[1])) {
            (Some(a), Some(b)) if a == b => {
                eprintln!("converging pair CO-LOCATED on {a}");
            }
            (a, b) => failures.push(format!(
                "converging pair NOT co-located at end: {a:?} vs {b:?}"
            )),
        }
        let pair_flips: usize = player_entities[..2]
            .iter()
            .filter_map(|id| tracks.get(id))
            .map(|t| t.flips.len())
            .sum();
        if pair_flips == 0 {
            failures.push(
                "converging pair co-location never required a flip — migration not exercised"
                    .to_string(),
            );
        } else {
            eprintln!("converging pair flips observed: {pair_flips}");
        }
    }

    // D2: in migrate phase with CONNECTED players, at least one player must
    // have followed a RECONNECT to the new owner — proving the full loop:
    // flip → forwarding → hint → make-before-break → direct path restored.
    let reconnects = shared
        .reconnects_followed
        .load(std::sync::atomic::Ordering::Relaxed);
    eprintln!("reconnects followed: {reconnects}");
    if args.phase == Phase::Migrate && !args.disconnect_at_target && reconnects == 0 {
        failures.push(
            "no player followed a RECONNECT hint — D2 redirect path not exercised".to_string(),
        );
    }

    if failures.is_empty() {
        eprintln!(
            "VERDICT: PASS — {} players, {} clusters, phase={}",
            args.players,
            args.clusters.len(),
            match args.phase {
                Phase::Static => "static",
                Phase::Migrate => "migrate",
                Phase::Restart => "restart",
                Phase::Cluster => "cluster",
                Phase::Defector => "defector",
                Phase::Gradient => "gradient",
                Phase::SpectrumIdle => "spectrum-idle",
                Phase::SpectrumWarmup => "spectrum-warmup",
            }
        );
    } else {
        eprintln!("VERDICT: FAIL");
        for f in &failures {
            eprintln!("  - {f}");
        }
        std::process::exit(1);
    }
}
