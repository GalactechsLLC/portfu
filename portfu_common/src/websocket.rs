use crate::error::PortfuError;
use crate::server::Server;
use crate::server::connection::ConnectionInfo;
use crate::service::request::{Request, RequestType};
use crate::service::response::Response;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use http::StatusCode;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Debug;
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::time::timeout;
pub use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::Utf8Bytes;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use uuid::Uuid;

pub type ServerWebSocketInner =
    tokio_tungstenite::WebSocketStream<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>;
pub type ClientWebSocketInner =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub type Peers = Arc<RwLock<HashMap<Uuid, Arc<WebsocketConnection>>>>;

#[derive(Clone, Copy, Debug)]
pub struct WebSocketRouteConfig {
    pub max_message_size: Option<usize>,
    pub max_frame_size: Option<usize>,
    pub upgrade_timeout: Option<Duration>,
}

impl Default for WebSocketRouteConfig {
    fn default() -> Self {
        let tungstenite = WebSocketConfig::default();
        Self {
            max_message_size: tungstenite.max_message_size,
            max_frame_size: tungstenite.max_frame_size,
            upgrade_timeout: None,
        }
    }
}

trait PermitValue: Send + Sync {}
impl<T: Send + Sync> PermitValue for T {}

pub struct WebSocketAdmissionPermit {
    _permit: Box<dyn PermitValue>,
}

impl WebSocketAdmissionPermit {
    pub fn new<T: Send + Sync + 'static>(permit: T) -> Self {
        Self {
            _permit: Box::new(permit),
        }
    }
}

impl Default for WebSocketAdmissionPermit {
    fn default() -> Self {
        Self::new(())
    }
}

pub enum WebSocketAdmission {
    Accept(WebSocketAdmissionPermit),
    Reject(Response),
}

pub trait WebSocketAdmissionMiddleware {
    fn name(&self) -> &str;
    fn admit<'a>(
        &'a self,
        request: &'a mut Request,
        connection: &'a ConnectionInfo,
    ) -> Pin<Box<dyn Future<Output = Result<WebSocketAdmission, PortfuError>> + Send + Sync + 'a>>;
}

pub async fn upgrade<F, Fut, E>(
    request: &mut Request,
    config: WebSocketRouteConfig,
    peers: Peers,
    handler: F,
) -> Result<Response, PortfuError>
where
    F: FnOnce(WebSocket) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), E>> + Send + 'static,
    E: Debug + Send + 'static,
{
    let server = request.get::<Arc<Server>>().cloned().ok_or_else(|| {
        PortfuError::Internal("WebSocket upgrade requires server runtime state".to_string())
    })?;
    let connection = request.get::<ConnectionInfo>().cloned().ok_or_else(|| {
        PortfuError::Internal("WebSocket upgrade requires ConnectionInfo".to_string())
    })?;

    let (key, version_ok, upgrade_ok, connection_ok) = match request.request_type() {
        RequestType::Stream(req) => websocket_headers(req.headers()),
        RequestType::Sized(req) => websocket_headers(req.headers()),
        _ => {
            return Ok(Response::from_status_and_message(
                StatusCode::BAD_REQUEST,
                "WebSocket upgrade requires a live HTTP request",
            ));
        }
    };
    if !upgrade_ok || !connection_ok {
        return Ok(Response::from_status_and_message(
            StatusCode::BAD_REQUEST,
            "Expected websocket upgrade request",
        ));
    }
    let Some(key) = key else {
        return Ok(Response::from_status_and_message(
            StatusCode::BAD_REQUEST,
            "Missing Sec-WebSocket-Key header",
        ));
    };
    if !version_ok {
        return Ok(Response::from_status_and_message(
            StatusCode::BAD_REQUEST,
            "Unsupported websocket version",
        ));
    }
    if server.runtime.is_shutting_down() {
        return Ok(Response::from_status_and_message(
            StatusCode::SERVICE_UNAVAILABLE,
            "Server is shutting down",
        ));
    }

    let mut permits = Vec::with_capacity(server.websocket_admission.len());
    for middleware in &server.websocket_admission {
        match middleware.admit(request, &connection).await? {
            WebSocketAdmission::Accept(permit) => permits.push(permit),
            WebSocketAdmission::Reject(response) => return Ok(response),
        }
    }

    let upgrade = match request.request_type() {
        RequestType::Stream(req) => hyper::upgrade::on(req),
        RequestType::Sized(req) => hyper::upgrade::on(req),
        _ => unreachable!("live request was checked above"),
    };
    let response = http::Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(http::header::CONNECTION, "upgrade")
        .header(http::header::UPGRADE, "websocket")
        .header("Sec-WebSocket-Accept", derive_accept_key(key.as_bytes()))
        .body(())
        .map_err(|e| PortfuError::Internal(format!("Failed to build websocket response: {e}")))?;

    let cancellation = server.runtime.cancellation();
    server.runtime.spawn_websocket(async move {
        let _permits = permits;
        let upgraded = tokio::select! {
            _ = cancellation.cancelled() => return,
            result = wait_for_upgrade(upgrade, config.upgrade_timeout) => match result {
                Ok(upgraded) => upgraded,
                Err(error) => {
                    log::error!("WebSocket upgrade failed: {error}");
                    return;
                }
            }
        };
        let tungstenite_config = WebSocketConfig::default()
            .max_message_size(config.max_message_size)
            .max_frame_size(config.max_frame_size);
        let websocket = tokio_tungstenite::WebSocketStream::from_raw_socket(
            hyper_util::rt::TokioIo::new(upgraded),
            Role::Server,
            Some(tungstenite_config),
        )
        .await;
        let websocket = WebSocket::with_peers(websocket, peers, connection).await;
        tokio::select! {
            _ = cancellation.cancelled() => {
                let _ = timeout(Duration::from_secs(1), websocket.close()).await;
            }
            result = handler(websocket.clone()) => {
                if let Err(error) = result {
                    log::error!("WebSocket handler exited with error: {error:?}");
                }
            }
        }
        let _ = websocket.leave().await;
    });

    Ok(response.into())
}

fn websocket_headers(headers: &http::HeaderMap) -> (Option<http::HeaderValue>, bool, bool, bool) {
    let key = headers.get("Sec-WebSocket-Key").cloned();
    let version_ok = headers
        .get("Sec-WebSocket-Version")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == "13");
    let upgrade_ok = headers
        .get(http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    let connection_ok = headers
        .get(http::header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split([' ', ','])
                .any(|token| token.eq_ignore_ascii_case("upgrade"))
        });
    (key, version_ok, upgrade_ok, connection_ok)
}

async fn wait_for_upgrade(
    upgrade: hyper::upgrade::OnUpgrade,
    upgrade_timeout: Option<Duration>,
) -> Result<hyper::upgrade::Upgraded, String> {
    match upgrade_timeout {
        Some(duration) => timeout(duration, upgrade)
            .await
            .map_err(|_| format!("timed out after {duration:?}"))?
            .map_err(|error| error.to_string()),
        None => upgrade.await.map_err(|error| error.to_string()),
    }
}

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
    connection_info: ConnectionInfo,
}

impl WebSocket {
    pub fn new(websocket: ServerWebSocketInner, connection_info: ConnectionInfo) -> Self {
        Self {
            connection: Arc::new(WebsocketConnection::new(websocket)),
            uuid: Arc::new(Uuid::new_v4()),
            peers: Arc::new(RwLock::new(HashMap::new())),
            connection_info,
        }
    }

    pub async fn with_peers(
        websocket: ServerWebSocketInner,
        peers: Peers,
        connection_info: ConnectionInfo,
    ) -> Self {
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
            connection_info,
        }
    }

    pub fn connection_info(&self) -> &ConnectionInfo {
        &self.connection_info
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

pub struct ClientWebsocketConnection {
    pub write: Mutex<SplitSink<ClientWebSocketInner, Message>>,
    pub read: Mutex<SplitStream<ClientWebSocketInner>>,
}

impl ClientWebsocketConnection {
    pub fn new(websocket: ClientWebSocketInner) -> Self {
        let (write, read) = websocket.split();
        Self {
            write: Mutex::new(write),
            read: Mutex::new(read),
        }
    }
}

/// An outbound websocket connection independent from server peer management.
#[derive(Clone)]
pub struct ClientWebSocket {
    pub connection: Arc<ClientWebsocketConnection>,
}

impl ClientWebSocket {
    pub fn new(websocket: ClientWebSocketInner) -> Self {
        Self {
            connection: Arc::new(ClientWebsocketConnection::new(websocket)),
        }
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

#[cfg(test)]
#[path = "../tests/unit/websocket.rs"]
mod tests;
