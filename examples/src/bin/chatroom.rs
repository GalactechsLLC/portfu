use log::LevelFilter;
pub use portfu::prelude::*;
use simple_logger::SimpleLogger;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock;

#[static_files("chatroom_assets", name = "chatroom-ui")]
pub struct ChatUi;

#[derive(Default)]
pub struct ChatHub {
    next_id: AtomicUsize,
    peers: RwLock<HashMap<usize, WebSocket>>,
}

#[tokio::main(flavor = "multi_thread")]
pub async fn main() -> Result<(), PortfuError> {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .env()
        .init()
        .unwrap();

    ServerBuilder::from_env()
        .host("0.0.0.0")
        .global_state(ChatHub::default())
        .build()
        .run()
        .await
}

#[get("/api/health")]
pub async fn health() -> Result<String, PortfuError> {
    Ok("ok".to_string())
}

#[websocket("/ws/chat/{name}")]
pub async fn chat_socket(
    name: Path,
    websocket: WebSocket,
    hub: State<ChatHub>,
) -> Result<(), PortfuError> {
    let id = hub.next_id.fetch_add(1, Ordering::Relaxed);
    let user_name = name.inner();

    {
        let mut peers = hub.peers.write().await;
        peers.insert(id, websocket.clone());
    }
    broadcast(&hub, Message::text(format!("system: {user_name} joined"))).await;

    loop {
        let incoming = websocket
            .next_message()
            .await
            .map_err(|e| PortfuError::Internal(format!("Failed to read websocket message: {e}")))?;
        let Some(msg) = incoming else {
            break;
        };
        if msg.is_close() {
            break;
        }
        if msg.is_text() {
            let text = msg.to_text().unwrap_or_default();
            broadcast(&hub, Message::text(format!("{user_name}: {text}"))).await;
        }
    }

    {
        let mut peers = hub.peers.write().await;
        peers.remove(&id);
    }
    broadcast(&hub, Message::text(format!("system: {user_name} left"))).await;
    Ok(())
}

async fn broadcast(hub: &State<ChatHub>, message: Message) {
    let peers = hub.peers.read().await;
    let sockets: Vec<WebSocket> = peers.values().cloned().collect();
    drop(peers);
    for socket in sockets {
        let _ = socket.send(message.clone()).await;
    }
}
