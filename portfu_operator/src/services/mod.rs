use crate::services::kube::*;
use portfu::macros::websocket;
use portfu::prelude::tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use portfu::prelude::{ServerBuilder, WebSocket};
use std::io::Error;

pub mod kube;

pub fn register_services(server: ServerBuilder) -> ServerBuilder {
    server
        .register(get_nodes)
        .register(get_ingress)
        .register(get_services)
        .register(get_configs)
        .register(get_volume_claims)
        .register(get_pods)
        .register(get_volumes)
        .register(get_storage_classes)
        .register(get_namespaces)
        .register(test_socket {
            peers: Default::default(),
        })
}

#[websocket("test_socket")]
pub async fn test_socket(socket: WebSocket) -> Result<(), Error> {
    socket
        .broadcast(Message::Text(Utf8Bytes::from_static("Example Message")))
        .await?;
    socket.send(Message::Close(None)).await?;
    Ok(())
}
