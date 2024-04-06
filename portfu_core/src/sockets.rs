use futures_util::future::lazy;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use std::task::Poll;
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use uuid::Uuid;

pub type Peers = Arc<RwLock<HashMap<Uuid, Arc<WebsocketConnection>>>>;

pub struct WebsocketConnection {
    pub write: RwLock<SplitSink<WebSocketStream<TokioIo<Upgraded>>, Message>>,
    pub read: RwLock<SplitStream<WebSocketStream<TokioIo<Upgraded>>>>,
}
impl WebsocketConnection {
    pub fn new(websocket: WebSocketStream<TokioIo<Upgraded>>) -> Self {
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
    pub async fn next_message(&self) -> Result<Option<Message>, Error> {
        let mut stream = self.connection.read.write().await;
        lazy(|ctx| match (*stream).poll_next_unpin(ctx) {
            Poll::Pending => Ok(None),
            Poll::Ready(None) => Err(Error::new(ErrorKind::ConnectionAborted, "Stream Closed")),
            Poll::Ready(Some(v)) => v.map(Some).map_err(|e| {
                Error::new(
                    ErrorKind::Other,
                    format!("Failed to Read Websocket Message: {e:?}"),
                )
            }),
        })
        .await
    }
    pub async fn send(&self, msg: Message) -> Result<(), Error> {
        let mut stream = self.connection.write.write().await;
        stream.send(msg).await.map_err(|e| {
            Error::new(
                ErrorKind::Other,
                format!("Failed to Send Websocket Message: {e:?}"),
            )
        })
    }
    pub async fn send_to(&self, msg: Message, uuid: Uuid) -> Result<(), Error> {
        match self.peers.read().await.get(&uuid).cloned() {
            None => Err(Error::new(
                ErrorKind::NotFound,
                format!("Failed to find peer with id {uuid}"),
            )),
            Some(peer) => {
                let mut stream = peer.write.write().await;
                stream.send(msg).await.map_err(|e| {
                    Error::new(
                        ErrorKind::Other,
                        format!("Failed to Send Websocket Message: {e:?}"),
                    )
                })
            }
        }
    }
    pub async fn broadcast(&self, msg: Message) -> Result<(), Error> {
        let mut stream = self.connection.write.write().await;
        stream.send(msg.clone()).await.map_err(|e| {
            Error::new(
                ErrorKind::Other,
                format!("Failed to Send Websocket Message: {e:?}"),
            )
        })?;
        self.broadcast_others(msg).await
    }
    pub async fn broadcast_others(&self, msg: Message) -> Result<(), Error> {
        for peer in self.peers.read().await.values().cloned() {
            let mut stream = peer.write.write().await;
            stream.send(msg.clone()).await.map_err(|e| {
                Error::new(
                    ErrorKind::Other,
                    format!("Failed to Send Websocket Message: {e:?}"),
                )
            })?;
        }
        Ok(())
    }
}
