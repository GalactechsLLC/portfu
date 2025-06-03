use futures_util::future::lazy;
use futures_util::stream::{FusedStream, SplitSink, SplitStream};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

pub type Peers = Arc<RwLock<HashMap<Uuid, Arc<WebsocketConnection>>>>;

pub struct WebsocketConnection {
    pub write: RwLock<SplitSink<WebsocketMsgStream, Message>>,
    pub read: RwLock<SplitStream<WebsocketMsgStream>>,
}
impl WebsocketConnection {
    pub fn new(websocket: WebsocketMsgStream) -> Self {
        let (write, read) = websocket.split();
        Self {
            write: RwLock::new(write),
            read: RwLock::new(read),
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
    pub fn new(connection: WebsocketConnection, uuid: Arc<Uuid>) -> Self {
        Self {
            connection: Arc::new(connection),
            uuid,
            peers: Default::default(),
        }
    }
    pub async fn next_message(&self) -> Result<Option<Message>, Error> {
        let mut stream = self.connection.read.write().await;
        lazy(|ctx| match (*stream).poll_next_unpin(ctx) {
            Poll::Pending => Ok(None),
            Poll::Ready(None) => Err(Error::new(ErrorKind::ConnectionAborted, "Stream Closed")),
            Poll::Ready(Some(v)) => v
                .map(Some)
                .map_err(|e| Error::other(format!("Failed to Read Websocket Message: {e:?}"))),
        })
        .await
    }
    pub async fn next(&self) -> Result<Option<Message>, Error> {
        self.connection
            .read
            .write()
            .await
            .next()
            .await
            .transpose()
            .map_err(|e| Error::other(format!("Failed to Read Next Websocket Message: {e:?}")))
    }
    pub async fn send(&self, msg: Message) -> Result<(), Error> {
        let mut stream = self.connection.write.write().await;
        stream
            .send(msg.clone())
            .await
            .map_err(|e| Error::other(format!("Failed to Send Websocket Message: {e:?}")))
    }
    pub async fn send_to(&self, msg: Message, uuid: Uuid) -> Result<(), Error> {
        match self.peers.read().await.get(&uuid).cloned() {
            None => Err(Error::new(
                ErrorKind::NotFound,
                format!("Failed to find peer with id {uuid}"),
            )),
            Some(peer) => {
                let mut stream = peer.write.write().await;
                stream
                    .send(msg)
                    .await
                    .map_err(|e| Error::other(format!("Failed to Send Websocket Message: {e:?}")))
            }
        }
    }
    pub async fn send_all(&self, msgs: Vec<Message>) -> Result<(), Error> {
        for msg in msgs {
            if let Err(e) = self.connection.write.write().await.feed(msg).await {
                let _ = self.connection.write.write().await.flush().await;
                return Err(Error::other(format!(
                    "Failed to Send Websocket Message: {e:?}"
                )));
            }
        }
        self.connection
            .write
            .write()
            .await
            .flush()
            .await
            .map_err(|e| Error::other(format!("Failed to Send Websocket Message: {e:?}")))
    }
    pub async fn broadcast(&self, msg: Message) -> Result<(), Error> {
        let mut stream = self.connection.write.write().await;
        stream
            .send(msg.clone())
            .await
            .map_err(|e| Error::other(format!("Failed to Send Websocket Message: {e:?}")))?;
        self.broadcast_others(msg).await
    }
    pub async fn broadcast_others(&self, msg: Message) -> Result<(), Error> {
        for peer in self.peers.read().await.values().cloned() {
            let mut stream = peer.write.write().await;
            stream
                .send(msg.clone())
                .await
                .map_err(|e| Error::other(format!("Failed to Send Websocket Message: {e:?}")))?;
        }
        Ok(())
    }
}

pub enum WebsocketMsgStream {
    TokioIo(Box<WebSocketStream<TokioIo<Upgraded>>>),
    Tls(Box<WebSocketStream<MaybeTlsStream<TcpStream>>>),
}
impl Stream for WebsocketMsgStream {
    type Item = Result<Message, tokio_tungstenite::tungstenite::error::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            WebsocketMsgStream::TokioIo(ref mut s) => Pin::new(s).poll_next(cx),
            WebsocketMsgStream::Tls(ref mut s) => Pin::new(s).poll_next(cx),
        }
    }
}
impl FusedStream for WebsocketMsgStream {
    fn is_terminated(&self) -> bool {
        match self {
            WebsocketMsgStream::TokioIo(s) => s.is_terminated(),
            WebsocketMsgStream::Tls(s) => s.is_terminated(),
        }
    }
}
impl Sink<Message> for WebsocketMsgStream {
    type Error = tokio_tungstenite::tungstenite::error::Error;
    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.get_mut() {
            WebsocketMsgStream::TokioIo(ref mut s) => Pin::new(s).poll_ready(cx),
            WebsocketMsgStream::Tls(ref mut s) => Pin::new(s).poll_ready(cx),
        }
    }
    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        match self.get_mut() {
            WebsocketMsgStream::TokioIo(ref mut s) => Pin::new(s).start_send(item),
            WebsocketMsgStream::Tls(ref mut s) => Pin::new(s).start_send(item),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.get_mut() {
            WebsocketMsgStream::TokioIo(ref mut s) => Pin::new(s).poll_flush(cx),
            WebsocketMsgStream::Tls(ref mut s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.get_mut() {
            WebsocketMsgStream::TokioIo(ref mut s) => Pin::new(s).poll_close(cx),
            WebsocketMsgStream::Tls(ref mut s) => Pin::new(s).poll_close(cx),
        }
    }
}
