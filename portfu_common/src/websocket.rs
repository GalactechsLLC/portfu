use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
pub use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::Utf8Bytes;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use uuid::Uuid;

pub type ServerWebSocketInner =
    tokio_tungstenite::WebSocketStream<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>;
pub type ClientWebSocketInner =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub type Peers = Arc<RwLock<HashMap<Uuid, Arc<WebsocketConnection>>>>;

pub struct WebsocketConnection {
    pub write: Mutex<SplitSink<ServerWebSocketInner, Message>>,
    pub read: Mutex<SplitStream<ServerWebSocketInner>>,
}

impl WebsocketConnection {
    pub fn new(websocket: ServerWebSocketInner) -> Self {
        let (write, read) = websocket.split();
        Self {
            write: Mutex::new(write),
            read: Mutex::new(read),
        }
    }
}

#[derive(Clone)]
pub struct WebSocket {
    pub connection: Arc<WebsocketConnection>,
    pub uuid: Arc<Uuid>,
    pub peers: Peers,
}

impl WebSocket {
    pub fn new(websocket: ServerWebSocketInner) -> Self {
        Self {
            connection: Arc::new(WebsocketConnection::new(websocket)),
            uuid: Arc::new(Uuid::new_v4()),
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn with_peers(websocket: ServerWebSocketInner, peers: Peers) -> Self {
        let uuid = Arc::new(Uuid::new_v4());
        let connection = Arc::new(WebsocketConnection::new(websocket));
        peers
            .write()
            .await
            .insert(*uuid.as_ref(), connection.clone());
        Self {
            connection,
            uuid,
            peers,
        }
    }

    pub fn id(&self) -> Uuid {
        *self.uuid.as_ref()
    }

    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }

    pub async fn has_peer(&self, uuid: Uuid) -> bool {
        self.peers.read().await.contains_key(&uuid)
    }

    pub async fn peers(&self) -> Vec<Uuid> {
        self.peers.read().await.keys().copied().collect()
    }

    pub async fn peer_ids(&self) -> Vec<Uuid> {
        self.peers().await
    }

    pub async fn next_message(&self) -> Result<Option<Message>, Error> {
        self.connection
            .read
            .lock()
            .await
            .next()
            .await
            .transpose()
            .map_err(|e| Error::other(format!("Failed to read websocket message: {e:?}")))
    }

    pub async fn next(&self) -> Result<Option<Message>, Error> {
        self.next_message().await
    }

    pub async fn send(&self, message: Message) -> Result<(), Error> {
        self.connection
            .write
            .lock()
            .await
            .send(message)
            .await
            .map_err(|e| Error::other(format!("Failed to send websocket message: {e:?}")))
    }

    pub async fn send_text<T: AsRef<str>>(&self, text: T) -> Result<(), Error> {
        self.send(Message::Text(text.as_ref().to_string().into()))
            .await
    }

    pub async fn send_binary<T: Into<Vec<u8>>>(&self, bytes: T) -> Result<(), Error> {
        self.send(Message::Binary(bytes.into().into())).await
    }

    pub async fn send_json<T: Serialize>(&self, value: &T) -> Result<(), Error> {
        let json = serde_json::to_string(value).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to serialize JSON: {e}"),
            )
        })?;
        self.send(Message::Text(json.into())).await
    }

    pub async fn send_to(&self, message: Message, uuid: Uuid) -> Result<(), Error> {
        match self.peers.read().await.get(&uuid).cloned() {
            None => Err(Error::new(
                ErrorKind::NotFound,
                format!("Failed to find peer with id {uuid}"),
            )),
            Some(peer) => peer
                .write
                .lock()
                .await
                .send(message)
                .await
                .map_err(|e| Error::other(format!("Failed to send websocket message: {e:?}"))),
        }
    }

    pub async fn send_text_to<T: AsRef<str>>(&self, text: T, uuid: Uuid) -> Result<(), Error> {
        self.send_to(Message::Text(text.as_ref().to_string().into()), uuid)
            .await
    }

    pub async fn send_json_to<T: Serialize>(&self, value: &T, uuid: Uuid) -> Result<(), Error> {
        let json = serde_json::to_string(value).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to serialize JSON: {e}"),
            )
        })?;
        self.send_to(Message::Text(json.into()), uuid).await
    }

    pub async fn send_all(&self, messages: Vec<Message>) -> Result<(), Error> {
        let mut writer = self.connection.write.lock().await;
        for message in messages {
            if let Err(e) = writer.feed(message).await {
                let _ = writer.flush().await;
                return Err(Error::other(format!(
                    "Failed to send websocket message: {e:?}"
                )));
            }
        }
        writer
            .flush()
            .await
            .map_err(|e| Error::other(format!("Failed to send websocket message: {e:?}")))
    }

    pub async fn broadcast(&self, message: Message) -> Result<(), Error> {
        self.send(message.clone()).await?;
        self.broadcast_others(message).await
    }

    pub async fn broadcast_text<T: AsRef<str>>(&self, text: T) -> Result<(), Error> {
        self.broadcast(Message::Text(text.as_ref().to_string().into()))
            .await
    }

    pub async fn broadcast_json<T: Serialize>(&self, value: &T) -> Result<(), Error> {
        let json = serde_json::to_string(value).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to serialize JSON: {e}"),
            )
        })?;
        self.broadcast(Message::Text(json.into())).await
    }

    pub async fn broadcast_others(&self, message: Message) -> Result<(), Error> {
        let peers = self.peers.read().await;
        for (uuid, peer) in peers.iter() {
            if *uuid == *self.uuid.as_ref() {
                continue;
            }
            peer.write
                .lock()
                .await
                .send(message.clone())
                .await
                .map_err(|e| Error::other(format!("Failed to send websocket message: {e:?}")))?;
        }
        Ok(())
    }

    pub async fn broadcast_others_text<T: AsRef<str>>(&self, text: T) -> Result<(), Error> {
        self.broadcast_others(Message::Text(text.as_ref().to_string().into()))
            .await
    }

    pub async fn broadcast_others_json<T: Serialize>(&self, value: &T) -> Result<(), Error> {
        let json = serde_json::to_string(value).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to serialize JSON: {e}"),
            )
        })?;
        self.broadcast_others(Message::Text(json.into())).await
    }

    pub async fn ping<T: Into<Vec<u8>>>(&self, payload: T) -> Result<(), Error> {
        self.send(Message::Ping(payload.into().into())).await
    }

    pub async fn pong<T: Into<Vec<u8>>>(&self, payload: T) -> Result<(), Error> {
        self.send(Message::Pong(payload.into().into())).await
    }

    pub async fn close(&self) -> Result<(), Error> {
        self.send(Message::Close(None)).await
    }

    pub async fn close_with<T: Into<String>>(&self, code: u16, reason: T) -> Result<(), Error> {
        self.send(Message::Close(Some(CloseFrame {
            code: CloseCode::from(code),
            reason: Utf8Bytes::from(reason.into()),
        })))
        .await
    }

    pub async fn leave(&self) -> Option<Arc<WebsocketConnection>> {
        self.peers.write().await.remove(self.uuid.as_ref())
    }
}

#[derive(Clone)]
pub struct WebSocketClient {
    inner: Arc<Mutex<ClientWebSocketInner>>,
}

impl WebSocketClient {
    pub fn new(websocket: ClientWebSocketInner) -> Self {
        Self {
            inner: Arc::new(Mutex::new(websocket)),
        }
    }

    pub async fn next_message(&self) -> Result<Option<Message>, Error> {
        self.inner
            .lock()
            .await
            .next()
            .await
            .transpose()
            .map_err(|e| Error::other(format!("Failed to read websocket message: {e:?}")))
    }

    pub async fn next(&self) -> Result<Option<Message>, Error> {
        self.next_message().await
    }

    pub async fn send(&self, message: Message) -> Result<(), Error> {
        self.inner
            .lock()
            .await
            .send(message)
            .await
            .map_err(|e| Error::other(format!("Failed to send websocket message: {e:?}")))
    }

    pub async fn send_text<T: AsRef<str>>(&self, text: T) -> Result<(), Error> {
        self.send(Message::Text(text.as_ref().to_string().into()))
            .await
    }

    pub async fn send_binary<T: Into<Vec<u8>>>(&self, bytes: T) -> Result<(), Error> {
        self.send(Message::Binary(bytes.into().into())).await
    }

    pub async fn send_json<T: Serialize>(&self, value: &T) -> Result<(), Error> {
        let json = serde_json::to_string(value).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to serialize JSON: {e}"),
            )
        })?;
        self.send(Message::Text(json.into())).await
    }

    pub async fn ping<T: Into<Vec<u8>>>(&self, payload: T) -> Result<(), Error> {
        self.send(Message::Ping(payload.into().into())).await
    }

    pub async fn pong<T: Into<Vec<u8>>>(&self, payload: T) -> Result<(), Error> {
        self.send(Message::Pong(payload.into().into())).await
    }

    pub async fn close(&self) -> Result<(), Error> {
        self.send(Message::Close(None)).await
    }

    pub async fn close_with<T: Into<String>>(&self, code: u16, reason: T) -> Result<(), Error> {
        self.send(Message::Close(Some(CloseFrame {
            code: CloseCode::from(code),
            reason: Utf8Bytes::from(reason.into()),
        })))
        .await
    }
}
