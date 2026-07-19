//! WS cluster probe: connect to a node's client WebSocket, decode server
//! frames with the real wire codec, and print the distinct `cluster_id`s
//! carried by the broadcast entities. This is what drives per-cluster
//! nameplate colors in game clients, so it verifies "players from another
//! cluster are visibly attributed to it" without a game client.
//!
//! Usage: cargo run -p arcane-infra --example ws_cluster_probe -- ws://127.0.0.1:8080 10

use std::collections::HashMap;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "ws://127.0.0.1:8080".to_string());
    let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);

    let (mut socket, _) = tungstenite::connect(&url).expect("ws connect failed");
    println!("connected to {url}, sampling for {seconds}s...");

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut clusters: HashMap<String, u64> = HashMap::new();
    let mut frames = 0u64;

    while Instant::now() < deadline {
        let msg = match socket.read() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("ws read error: {e}");
                break;
            }
        };
        let tungstenite::Message::Binary(bytes) = msg else {
            continue;
        };
        let Ok(arcane_wire::ServerFrame::Delta(delta)) = arcane_wire::decode_server(&bytes) else {
            continue;
        };
        frames += 1;
        for entity in &delta.updated {
            *clusters
                .entry(entity.cluster_id.hyphenated().to_string())
                .or_default() += 1;
        }
    }

    println!("frames decoded: {frames}");
    println!("distinct owner cluster_ids on the wire: {}", clusters.len());
    let mut sorted: Vec<_> = clusters.into_iter().collect();
    sorted.sort();
    for (cluster, count) in sorted {
        println!("  {cluster}: {count} entity-updates");
    }
}
