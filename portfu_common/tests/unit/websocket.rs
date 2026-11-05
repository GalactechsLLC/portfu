use crate::error::PortfuError;
use crate::router::route::Route;
use crate::server::builder::ServerBuilder;
use crate::server::connection::ConnectionInfo;
use crate::service::request::{Request, RequestType};
use crate::service::response::Response;
use crate::websocket::{
    ClientWebSocket, Message, WebSocketAdmission, WebSocketAdmissionMiddleware,
    WebSocketRouteConfig, upgrade,
};
use futures_util::{SinkExt, StreamExt};
use http::StatusCode;
use http_body_util::Full;
use hyper::body::Bytes;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_tungstenite::{accept_async, connect_async};

#[derive(Serialize)]
struct JsonPayload {
    id: u64,
}

struct RejectAdmission;

impl WebSocketAdmissionMiddleware for RejectAdmission {
    fn name(&self) -> &str {
        "reject"
    }

    fn admit<'a>(
        &'a self,
        _request: &'a mut Request,
        _connection: &'a ConnectionInfo,
    ) -> Pin<Box<dyn Future<Output = Result<WebSocketAdmission, PortfuError>> + Send + Sync + 'a>>
    {
        Box::pin(async {
            Ok(WebSocketAdmission::Reject(
                Response::from_status_and_message(StatusCode::FORBIDDEN, "banned"),
            ))
        })
    }
}

#[tokio::test]
async fn admission_can_reject_before_switching_protocols() {
    let server = Arc::new(
        ServerBuilder::new()
            .websocket_admission(Arc::new(RejectAdmission))
            .build(),
    );
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);
    let connection = ConnectionInfo::plaintext(address, address);
    let raw = http::Request::builder()
        .uri("/ws")
        .header(http::header::CONNECTION, "upgrade")
        .header(http::header::UPGRADE, "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let mut request = Request::new(
        RequestType::Sized(raw),
        Arc::new(Route::new("/ws".to_string())),
    );
    request.insert(server);
    request.insert(connection);

    let response = upgrade(
        &mut request,
        WebSocketRouteConfig::default(),
        Arc::new(RwLock::new(HashMap::new())),
        |_| async { Ok::<(), PortfuError>(()) },
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn upgrade_rejects_missing_connection_upgrade_token() {
    let server = Arc::new(ServerBuilder::new().build());
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);
    let connection = ConnectionInfo::plaintext(address, address);
    let raw = http::Request::builder()
        .uri("/ws")
        .header(http::header::UPGRADE, "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let mut request = Request::new(
        RequestType::Sized(raw),
        Arc::new(Route::new("/ws".to_string())),
    );
    request.insert(server);
    request.insert(connection);

    let response = upgrade(
        &mut request,
        WebSocketRouteConfig::default(),
        Arc::new(RwLock::new(HashMap::new())),
        |_| async { Ok::<(), PortfuError>(()) },
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn client_websocket_wraps_connect_async_and_proxies_messages() {
    let Some(listener) = bind_listener_or_skip().await else {
        return;
    };
    let addr = listener.local_addr().expect("local addr failed");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept failed");
        let mut websocket = accept_async(stream).await.expect("accept websocket failed");
        while let Some(message) = websocket.next().await {
            let message = message.expect("websocket read failed");
            if message.is_close() {
                break;
            }
            websocket
                .send(message)
                .await
                .expect("websocket echo failed");
        }
    });

    let (stream, _) = connect_async(format!("ws://{addr}"))
        .await
        .expect("client connect failed");
    let client = ClientWebSocket::new(stream);

    client.send_text("hello").await.expect("send text failed");
    match client.next().await.expect("text read failed") {
        Some(Message::Text(text)) => assert_eq!(text, "hello"),
        other => panic!("unexpected text frame: {other:?}"),
    }

    client
        .send_binary([1_u8, 2, 3])
        .await
        .expect("send binary failed");
    match client.next_message().await.expect("binary read failed") {
        Some(Message::Binary(bytes)) => assert_eq!(bytes.as_ref(), &[1, 2, 3]),
        other => panic!("unexpected binary frame: {other:?}"),
    }

    client
        .send_json(&JsonPayload { id: 7 })
        .await
        .expect("send json failed");
    match client.next().await.expect("json read failed") {
        Some(Message::Text(text)) => assert_eq!(text, r#"{"id":7}"#),
        other => panic!("unexpected json frame: {other:?}"),
    }

    client.ping("ping").await.expect("send ping failed");
    client.pong("pong").await.expect("send pong failed");
    client
        .close_with(1000, "done")
        .await
        .expect("close frame failed");
    server.await.expect("server task failed");
}

#[tokio::test]
async fn websocket_client_reports_json_serialization_failures() {
    let Some(listener) = bind_listener_or_skip().await else {
        return;
    };
    let addr = listener.local_addr().expect("local addr failed");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept failed");
        let _websocket = accept_async(stream).await.expect("accept websocket failed");
    });

    let (stream, _) = connect_async(format!("ws://{addr}"))
        .await
        .expect("client connect failed");
    let client = ClientWebSocket::new(stream);
    let mut invalid_json_key = BTreeMap::new();
    invalid_json_key.insert(vec![1_u8, 2, 3], "value");

    let err = client
        .send_json(&invalid_json_key)
        .await
        .expect_err("non-string json object key should fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("Failed to serialize JSON"));

    client.close().await.expect("close frame failed");
    server.await.expect("server task failed");
}

async fn bind_listener_or_skip() -> Option<TcpListener> {
    match StdTcpListener::bind("127.0.0.1:0") {
        Ok(listener) => {
            listener
                .set_nonblocking(true)
                .expect("set nonblocking failed");
            Some(TcpListener::from_std(listener).expect("tokio listener conversion failed"))
        }
        Err(err) if err.kind() == ErrorKind::PermissionDenied => None,
        Err(err) => panic!("listener bind failed: {err}"),
    }
}
