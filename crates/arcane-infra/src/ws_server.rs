//! WebSocket server for cluster state broadcast. Built only with feature "cluster-ws".
//! Accepts incoming PLAYER_STATE messages from clients and forwards them to the tick loop.

use std::net::SocketAddr;
use std::sync::mpsc::{Receiver, Sender};

use arcane_core::replication_channel::{EntityStateDelta, EntityStateEntry};
use arcane_core::Vec3;
use futures_util::{sink::SinkExt, stream::StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// Incoming message from a client. Expects JSON: {"type":"PLAYER_STATE","entity_id":"uuid","position":{"x",y,z},"velocity":{"x",y,z}}
#[derive(serde::Deserialize)]
struct PlayerStateMessage {
    #[serde(rename = "type")]
    msg_type: String,
    entity_id: String,
    position: Vec3Message,
    velocity: Vec3Message,
}

#[derive(serde::Deserialize)]
struct Vec3Message {
    x: f64,
    y: f64,
    z: f64,
}

fn parse_player_state(text: &str) -> Option<EntityStateEntry> {
    let msg: PlayerStateMessage = serde_json::from_str(text).ok()?;
    if msg.msg_type != "PLAYER_STATE" {
        return None;
    }
    let entity_id = Uuid::parse_str(&msg.entity_id).ok()?;
    // cluster_id set by cluster binary when applying (this connection is to that cluster)
    Some(EntityStateEntry {
        entity_id,
        cluster_id: Uuid::nil(),
        position: Vec3::new(msg.position.x, msg.position.y, msg.position.z),
        velocity: Vec3::new(msg.velocity.x, msg.velocity.y, msg.velocity.z),
    })
}

pub fn run_ws_server(
    port: u16,
    state_rx: Receiver<EntityStateDelta>,
    client_updates_tx: Sender<EntityStateEntry>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(ws_loop(port, state_rx, client_updates_tx));
    });
}

async fn ws_loop(
    port: u16,
    state_rx: Receiver<EntityStateDelta>,
    client_updates_tx: Sender<EntityStateEntry>,
) {
    let (broadcast_tx, _) = tokio::sync::broadcast::channel::<String>(256);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await.expect("bind ws port");
    eprintln!(
        "cluster WebSocket listening on ws://{} (send PLAYER_STATE to push player entity)",
        addr
    );

    let broadcast_tx = std::sync::Arc::new(broadcast_tx);
    let tx_clone = broadcast_tx.clone();
    let rx = std::sync::Arc::new(std::sync::Mutex::new(state_rx));
    tokio::spawn(async move {
        loop {
            let r = rx.clone();
            let delta = tokio::task::spawn_blocking(move || r.lock().unwrap().recv())
                .await
                .unwrap();
            match delta {
                Ok(d) => {
                    if let Ok(json) = serde_json::to_string(&d) {
                        let _ = tx_clone.send(json);
                    }
                }
                Err(_) => break,
            }
        }
    });

    while let Ok((stream, _)) = listener.accept().await {
        let mut ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut recv = broadcast_tx.subscribe();
        let updates_tx = client_updates_tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = recv.recv() => {
                        match result {
                            Ok(json) => {
                                if ws_stream.send(Message::Text(json)).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    msg = ws_stream.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Some(entry) = parse_player_state(&text) {
                                    let _ = updates_tx.send(entry);
                                }
                            }
                            Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                }
            }
        });
    }
}
