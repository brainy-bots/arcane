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
    /// (observer, from, to, position_delta) attribution changes.
    flips: Vec<(usize, Uuid, Uuid, f64)>,
    max_jump_seen: f64,
    max_gap_ms_seen: u128,
}

struct Shared {
    tracks: Mutex<HashMap<Uuid, EntityTrack>>,
    stop: AtomicBool,
    /// D2: RECONNECT frames followed by players (make-before-break moves).
    reconnects_followed: std::sync::atomic::AtomicU64,
}

fn dist(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    let dz = a.2 - b.2;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn observer_thread(idx: usize, ws_url: String, shared: Arc<Shared>) {
    let (mut socket, _) = match tungstenite::connect(&ws_url) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[obs {idx}] connect {ws_url} failed: {e}");
            return;
        }
    };
    eprintln!("[obs {idx}] connected to {ws_url}");
    while !shared.stop.load(Ordering::Relaxed) {
        let msg = match socket.read() {
            Ok(m) => m,
            Err(_) => break,
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
                let jump = dist(prev.position, obs.position);
                if prev.cluster_id != obs.cluster_id {
                    track
                        .flips
                        .push((idx, prev.cluster_id, obs.cluster_id, jump));
                } else if jump > track.max_jump_seen {
                    track.max_jump_seen = jump;
                }
                let gap = now.duration_since(prev.at).as_millis();
                if gap > track.max_gap_ms_seen {
                    track.max_gap_ms_seen = gap;
                }
            }
            track.latest.insert(idx, obs);
        }
    }
    eprintln!("[obs {idx}] done");
}

struct PlayerSpec {
    idx: u32,
    spawn: (f64, f64),
    /// Walk toward this point at `speed` server-units/sec (None = static).
    target: Option<(f64, f64)>,
    speed: f64,
    /// Legacy pinned-stack mode: stop sending and close the socket on
    /// reaching the target so the entity becomes plain server-side state.
    /// Default (false) since D1: the player STAYS CONNECTED through the
    /// flip and the forwarding invariant keeps it single-writer correct.
    disconnect_at_target: bool,
}

fn spawn_player(spec: PlayerSpec, manager: &str, shared: Arc<Shared>) -> Option<Uuid> {
    let idx = spec.idx;
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
        while !shared.stop.load(Ordering::Relaxed) {
            // Movement toward the target, if any.
            let (mut vx, mut vz) = (0.0, 0.0);
            let mut arrived = false;
            if let Some((tx, tz)) = spec.target {
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

fn main() {
    let args = parse_args();
    let shared = Arc::new(Shared {
        tracks: Mutex::new(HashMap::new()),
        stop: AtomicBool::new(false),
        reconnects_followed: std::sync::atomic::AtomicU64::new(0),
    });

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
        let spec = match (args.phase, i) {
            (Phase::Migrate, 0) => PlayerSpec {
                idx: i,
                spawn: (100.0, 100.0),
                target: Some(convergence),
                speed: 60.0,
                disconnect_at_target: args.disconnect_at_target,
            },
            (Phase::Migrate, 1) => PlayerSpec {
                idx: i,
                spawn: (3000.0, 3000.0),
                target: Some(convergence),
                speed: 60.0,
                disconnect_at_target: args.disconnect_at_target,
            },
            (Phase::Migrate, _) => PlayerSpec {
                // Static bystanders live in far corners, away from the
                // convergence point: no proximity edges, no partition
                // pressure, no reason to migrate.
                idx: i,
                spawn: (5000.0 + 800.0 * i as f64, 200.0),
                target: None,
                speed: 0.0,
                disconnect_at_target: false,
            },
            _ => PlayerSpec {
                idx: i,
                spawn: (100.0 + 700.0 * i as f64, 100.0 + 700.0 * i as f64),
                target: None,
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
        if owners.len() > 1 && !transitioning {
            failures.push(format!(
                "player {i} ({id}): observers DISAGREE on owner: {owners:?}"
            ));
        }
        // Flips: forbidden in static phase (pinned); expected only for the
        // converging pair in migrate phase.
        let flips_allowed = args.phase == Phase::Migrate;
        if !flips_allowed && !t.flips.is_empty() {
            failures.push(format!(
                "player {i} ({id}): {} attribution flip(s) despite pinning",
                t.flips.len()
            ));
        }
        // Every flip must be position-continuous (the same entity may not
        // teleport when its owner changes — the §8 adoption seed guarantees it).
        for (obs, from, to, jump) in &t.flips {
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
        if t.max_gap_ms_seen > args.max_gap_ms as u128 {
            failures.push(format!(
                "player {i} ({id}): update gap {}ms > {}ms (stale/slow class)",
                t.max_gap_ms_seen, args.max_gap_ms
            ));
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
