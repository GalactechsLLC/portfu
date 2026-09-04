use futures_util::{SinkExt, StreamExt};
use http::Method;
use http_body_util::BodyExt;
use portfu::prelude::*;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, date_time_ymd,
};
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener as TokioTcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{accept_async, connect_async};

fn method_response(
    method: &str,
    body: &str,
) -> Result<http::Response<http_body_util::Full<hyper::body::Bytes>>, PortfuError> {
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("x-method", method)
        .header("x-content-check", body.len().to_string())
        .body(http_body_util::Full::new(hyper::body::Bytes::from(
            body.to_string(),
        )))
        .map_err(|e| PortfuError::Internal(format!("failed to build response: {e}")))
}

#[get("/method/get")]
async fn method_get()
-> Result<http::Response<http_body_util::Full<hyper::body::Bytes>>, PortfuError> {
    method_response("GET", "get-ok")
}

#[post("/method/post")]
async fn method_post()
-> Result<http::Response<http_body_util::Full<hyper::body::Bytes>>, PortfuError> {
    method_response("POST", "post-ok")
}

#[put("/method/put")]
async fn method_put()
-> Result<http::Response<http_body_util::Full<hyper::body::Bytes>>, PortfuError> {
    method_response("PUT", "put-ok")
}

#[delete("/method/delete")]
async fn method_delete()
-> Result<http::Response<http_body_util::Full<hyper::body::Bytes>>, PortfuError> {
    method_response("DELETE", "delete-ok")
}

#[patch("/method/patch")]
async fn method_patch()
-> Result<http::Response<http_body_util::Full<hyper::body::Bytes>>, PortfuError> {
    method_response("PATCH", "patch-ok")
}

#[trace("/method/trace")]
async fn method_trace()
-> Result<http::Response<http_body_util::Full<hyper::body::Bytes>>, PortfuError> {
    method_response("TRACE", "trace-ok")
}

#[connect("/method/connect")]
async fn method_connect()
-> Result<http::Response<http_body_util::Full<hyper::body::Bytes>>, PortfuError> {
    method_response("CONNECT", "connect-ok")
}

#[options("/method/options")]
async fn method_options()
-> Result<http::Response<http_body_util::Full<hyper::body::Bytes>>, PortfuError> {
    method_response("OPTIONS", "options-ok")
}

#[head("/method/head")]
async fn method_head() -> Result<http::Response<()>, PortfuError> {
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("x-method", "HEAD")
        .body(())
        .map_err(|e| PortfuError::Internal(format!("failed to build head response: {e}")))
}

static SLOW_REQUEST_STARTED: tokio::sync::Notify = tokio::sync::Notify::const_new();

#[get("/method/slow")]
async fn method_slow() -> Result<String, PortfuError> {
    SLOW_REQUEST_STARTED.notify_one();
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok("slow-ok".to_string())
}

#[get("/tls/open", name = "tls-open")]
async fn tls_open() -> Result<String, PortfuError> {
    Ok("open".to_string())
}

#[get("/tls/public", name = "tls-public", client_trust = "public-clients")]
async fn tls_public(identity: ClientIdentity) -> Result<String, PortfuError> {
    if !identity
        .verified_by
        .iter()
        .any(|name| name == "public-clients")
    {
        return Err(PortfuError::Internal(
            "public client identity was not propagated".to_string(),
        ));
    }
    Ok("public".to_string())
}

#[get(
    "/tls/internal",
    name = "tls-internal",
    client_trust = "internal-clients"
)]
async fn tls_internal(identity: ClientIdentity) -> Result<String, PortfuError> {
    if !identity
        .verified_by
        .iter()
        .any(|name| name == "internal-clients")
    {
        return Err(PortfuError::Internal(
            "internal client identity was not propagated".to_string(),
        ));
    }
    Ok("internal".to_string())
}

#[websocket(
    "/ws/echo",
    max_message_size = 67_108_864,
    max_frame_size = 16_777_216,
    upgrade_timeout_ms = 5_000
)]
async fn ws_echo(info: ConnectionInfo, websocket: WebSocket) -> Result<(), PortfuError> {
    if websocket.connection_info() != &info {
        return Err(PortfuError::Internal(
            "connection metadata changed during upgrade".to_string(),
        ));
    }
    while let Some(message) = websocket
        .next_message()
        .await
        .map_err(|e| PortfuError::Internal(format!("websocket read failed: {e}")))?
    {
        if message.is_close() {
            break;
        }
        websocket
            .send(message)
            .await
            .map_err(|e| PortfuError::Internal(format!("websocket send failed: {e}")))?;
    }
    Ok(())
}

#[websocket(
    "/ws/limited",
    max_message_size = 8,
    max_frame_size = 8,
    upgrade_timeout_ms = 1_000
)]
async fn ws_limited(websocket: WebSocket) -> Result<(), PortfuError> {
    let _ = websocket
        .next_message()
        .await
        .map_err(|e| PortfuError::BadRequest(format!("websocket limit enforced: {e}")))?;
    Ok(())
}

#[websocket("/ws/owned-headers")]
async fn ws_owned_headers(
    headers: RequestHeaders,
    websocket: WebSocket,
) -> Result<(), PortfuError> {
    let _requested_protocol = headers.get("sec-websocket-protocol");
    websocket
        .close()
        .await
        .map_err(|e| PortfuError::Internal(format!("websocket close failed: {e}")))
}

#[client_websocket("ws://127.0.0.1:38481")]
async fn macro_client_echo(websocket: ClientWebSocket) -> Result<(), PortfuError> {
    websocket
        .send(Message::text("macro echo"))
        .await
        .map_err(|e| PortfuError::Internal(format!("websocket send failed: {e}")))?;
    match websocket
        .next_message()
        .await
        .map_err(|e| PortfuError::Internal(format!("websocket read failed: {e}")))?
    {
        Some(Message::Text(message)) if message == "macro echo" => Ok(()),
        other => Err(PortfuError::Internal(format!(
            "unexpected websocket response: {other:?}"
        ))),
    }
}

#[derive(Debug)]
struct RawResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct TestCa {
    certificate: Certificate,
    key: KeyPair,
}

impl TestCa {
    fn new() -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        let key = KeyPair::generate().unwrap();
        let certificate = params.self_signed(&key).unwrap();
        Self { certificate, key }
    }

    fn issue_client(&self, expired: bool) -> TestClientCertificate {
        let mut params = CertificateParams::new(vec!["client.test".to_string()]).unwrap();
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        if expired {
            params.not_before = date_time_ymd(2020, 1, 1);
            params.not_after = date_time_ymd(2021, 1, 1);
        }
        let key = KeyPair::generate().unwrap();
        let certificate = params
            .signed_by(&key, &self.certificate, &self.key)
            .unwrap();
        TestClientCertificate {
            certificate: certificate.der().clone(),
            private_key: key.serialize_der(),
        }
    }
}

struct TestClientCertificate {
    certificate: CertificateDer<'static>,
    private_key: Vec<u8>,
}

struct TestServer {
    port: u16,
    handle: ServerHandle,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    async fn start() -> Option<Self> {
        let port = match reserve_port() {
            Ok(port) => port,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                eprintln!(
                    "skipping e2e server test: local socket bind is not permitted in this environment: {err}"
                );
                return None;
            }
            Err(err) => panic!("failed to reserve server test port: {err}"),
        };
        let (handle_tx, handle_rx) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime for test server");
            runtime.block_on(async move {
                let server = ServerBuilder::from_env()
                    .host("127.0.0.1")
                    .port(port)
                    .build();
                let _ = handle_tx.send(server.handle());
                server.run().await.expect("test server failed");
            });
        });

        let handle = handle_rx.await.expect("test server did not publish handle");

        wait_for_server(port)
            .await
            .expect("server failed to become ready");

        Some(Self {
            port,
            handle,
            thread: Some(thread),
        })
    }

    async fn stop(mut self) {
        self.handle.shutdown();
        if let Some(thread) = self.thread.take() {
            tokio::task::spawn_blocking(move || {
                let _ = thread.join();
            })
            .await
            .expect("failed joining server thread");
        }
    }

    async fn start_tls(tls: TlsConfig) -> Option<Self> {
        let port = match reserve_port() {
            Ok(port) => port,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                eprintln!("skipping TLS e2e test: local socket bind is not permitted: {err}");
                return None;
            }
            Err(err) => panic!("failed to reserve TLS test port: {err}"),
        };
        let (handle_tx, handle_rx) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build TLS test runtime");
            runtime.block_on(async move {
                let mut builder = ServerBuilder::new().host("127.0.0.1").port(port).tls(tls);
                builder.config.acceptors = 1;
                builder.config.reuse_port = false;
                let server = builder.build();
                let _ = handle_tx.send(server.handle());
                server.run().await.expect("TLS test server failed");
            });
        });
        let handle = handle_rx.await.expect("TLS server did not publish handle");

        Some(Self {
            port,
            handle,
            thread: Some(thread),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_drains_in_flight_http_requests() {
    let Some(server) = TestServer::start().await else {
        return;
    };
    let port = server.port;
    let request = tokio::spawn(async move {
        raw_http_request(port, "GET", "/method/slow", None)
            .await
            .expect("slow request failed")
    });
    SLOW_REQUEST_STARTED.notified().await;

    server.stop().await;

    let response = request.await.expect("slow request task failed");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"slow-ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_closes_and_drains_websockets() {
    let Some(server) = TestServer::start().await else {
        return;
    };
    let (mut socket, response) = connect_async(format!("ws://127.0.0.1:{}/ws/echo", server.port))
        .await
        .expect("websocket connect failed");
    assert_eq!(response.status(), http::StatusCode::SWITCHING_PROTOCOLS);

    let shutdown = tokio::spawn(server.stop());
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("websocket did not close during shutdown");
    assert!(
        matches!(message, None | Some(Ok(WsMessage::Close(_)))),
        "unexpected websocket shutdown message: {message:?}"
    );
    shutdown.await.expect("server shutdown task failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_route_enforces_message_and_frame_limits() {
    let Some(server) = TestServer::start().await else {
        return;
    };
    let (mut socket, response) =
        connect_async(format!("ws://127.0.0.1:{}/ws/limited", server.port))
            .await
            .expect("websocket connect failed");
    assert_eq!(response.status(), http::StatusCode::SWITCHING_PROTOCOLS);

    socket
        .send(WsMessage::Text("message larger than eight bytes".into()))
        .await
        .expect("websocket send failed");
    let result = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("server did not terminate an oversized websocket");
    assert!(
        !matches!(result, Some(Ok(WsMessage::Text(_)))),
        "oversized websocket message was unexpectedly accepted"
    );

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_handshake_timeout_closes_idle_clients() {
    let port = match reserve_port() {
        Ok(port) => port,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return,
        Err(err) => panic!("failed to reserve TLS test port: {err}"),
    };
    let (handle_tx, handle_rx) = oneshot::channel();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build TLS test runtime");
        runtime.block_on(async move {
            let server = ServerBuilder::new()
                .host("127.0.0.1")
                .port(port)
                .tls(TlsConfig::default().handshake_timeout(Duration::from_millis(50)))
                .build();
            let _ = handle_tx.send(server.handle());
            server.run().await.expect("TLS test server failed");
        });
    });
    let handle = handle_rx.await.expect("TLS server did not publish handle");
    let mut stream = connect_with_retry(port)
        .await
        .expect("failed to connect idle TLS client");
    let mut byte = [0_u8; 1];
    let bytes = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
        .await
        .expect("idle TLS connection was not bounded")
        .expect("idle TLS socket read failed");
    assert_eq!(bytes, 0, "idle TLS connection remained open");

    handle.shutdown();
    tokio::task::spawn_blocking(move || thread.join().expect("TLS server thread panicked"))
        .await
        .expect("failed joining TLS server thread");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_certificate_handshakes_and_route_trust_are_enforced() {
    let server_identity =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let public_ca = TestCa::new();
    let internal_ca = TestCa::new();
    let unrelated_ca = TestCa::new();
    let public_client = public_ca.issue_client(false);
    let internal_client = internal_ca.issue_client(false);
    let unrelated_client = unrelated_ca.issue_client(false);
    let expired_client = public_ca.issue_client(true);

    let client_auth = ClientAuthConfig {
        presentation: ClientCertificateMode::Optional,
        trust_stores: vec![
            TrustStore::new("public-clients", public_ca.certificate.pem()),
            TrustStore::new("internal-clients", internal_ca.certificate.pem()),
        ],
    };
    let tls = TlsConfig::new(TlsIdentity::new(
        "localhost",
        server_identity.cert.pem(),
        server_identity.key_pair.serialize_pem(),
    ))
    .versions(TlsVersionPolicy::Tls13Only)
    .client_auth(client_auth.clone());
    let Some(server) = TestServer::start_tls(tls).await else {
        return;
    };

    let anonymous = Arc::new(tls_client_config(server_identity.cert.der(), None));
    let public = Arc::new(tls_client_config(
        server_identity.cert.der(),
        Some(&public_client),
    ));
    let internal = Arc::new(tls_client_config(
        server_identity.cert.der(),
        Some(&internal_client),
    ));
    let unrelated = Arc::new(tls_client_config(
        server_identity.cert.der(),
        Some(&unrelated_client),
    ));
    let expired = Arc::new(tls_client_config(
        server_identity.cert.der(),
        Some(&expired_client),
    ));

    assert_tls_response(server.port, anonymous.clone(), "/tls/open", 200, b"open").await;
    assert_tls_response(
        server.port,
        anonymous.clone(),
        "/tls/public",
        401,
        b"Client certificate required",
    )
    .await;
    assert_tls_response(server.port, public.clone(), "/tls/public", 200, b"public").await;
    assert_tls_response(server.port, public, "/tls/internal", 403, b"").await;
    assert_tls_response(
        server.port,
        internal.clone(),
        "/tls/internal",
        200,
        b"internal",
    )
    .await;
    assert_tls_response(server.port, internal, "/tls/public", 403, b"").await;

    assert!(
        tls_http_request(server.port, unrelated, "/tls/open")
            .await
            .is_err(),
        "a certificate from an unconfigured CA reached HTTP routing"
    );
    assert!(
        tls_http_request(server.port, expired, "/tls/open")
            .await
            .is_err(),
        "an expired client certificate reached HTTP routing"
    );
    server.stop().await;

    let required_tls = TlsConfig::new(TlsIdentity::new(
        "localhost",
        server_identity.cert.pem(),
        server_identity.key_pair.serialize_pem(),
    ))
    .versions(TlsVersionPolicy::Tls13Only)
    .client_auth(ClientAuthConfig {
        presentation: ClientCertificateMode::Required,
        ..client_auth
    });
    let Some(required_server) = TestServer::start_tls(required_tls).await else {
        return;
    };
    let valid = Arc::new(tls_client_config(
        server_identity.cert.der(),
        Some(&public_client),
    ));
    assert_tls_response(required_server.port, valid, "/tls/open", 200, b"open").await;
    assert!(
        tls_http_request(required_server.port, anonymous, "/tls/open")
            .await
            .is_err(),
        "required client authentication allowed a request without a certificate"
    );
    required_server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_methods_and_websocket_work_end_to_end() {
    let Some(server) = TestServer::start().await else {
        return;
    };
    let port = server.port;

    assert_method(port, "GET", "/method/get", "GET", "get-ok").await;
    assert_method(port, "POST", "/method/post", "POST", "post-ok").await;
    assert_method(port, "PUT", "/method/put", "PUT", "put-ok").await;
    assert_method(port, "DELETE", "/method/delete", "DELETE", "delete-ok").await;
    assert_method(port, "PATCH", "/method/patch", "PATCH", "patch-ok").await;
    assert_method(port, "TRACE", "/method/trace", "TRACE", "trace-ok").await;
    let options = raw_http_request(port, "OPTIONS", "/method/options", None)
        .await
        .expect("options request failed");
    assert_eq!(options.status, 200);
    assert_eq!(header(&options, "x-method"), None);
    assert!(options.body.is_empty());

    let head = raw_http_request(port, "HEAD", "/method/head", None)
        .await
        .expect("head request failed");
    assert_eq!(head.status, 200);
    assert_eq!(header(&head, "x-method"), Some("HEAD"));
    assert!(head.body.is_empty());

    let ws_url = format!("ws://127.0.0.1:{port}/ws/echo");
    let (mut socket, response) = connect_async(ws_url)
        .await
        .expect("websocket connect failed");
    assert_eq!(response.status(), http::StatusCode::SWITCHING_PROTOCOLS);
    socket
        .send(WsMessage::Text("echo this".into()))
        .await
        .expect("websocket send failed");
    match socket.next().await {
        Some(Ok(WsMessage::Text(msg))) => assert_eq!(msg, "echo this"),
        other => panic!("unexpected websocket response: {other:?}"),
    }
    let _ = socket.close(None).await;

    server.stop().await;
}

#[tokio::test]
async fn client_websocket_macro_connects_and_receives_echoed_data() {
    let listener = match TcpListener::bind(("127.0.0.1", 38481)) {
        Ok(listener) => listener,
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::PermissionDenied | ErrorKind::AddrInUse
            ) =>
        {
            eprintln!("skipping client websocket macro test: {err}");
            return;
        }
        Err(err) => panic!("failed to bind client websocket macro listener: {err}"),
    };
    listener
        .set_nonblocking(true)
        .expect("failed to make client websocket listener nonblocking");
    let listener = TokioTcpListener::from_std(listener)
        .expect("failed to convert client websocket listener to tokio");
    let echo_server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("client connection failed");
        let mut websocket = accept_async(stream)
            .await
            .expect("websocket handshake failed");
        let message = websocket
            .next()
            .await
            .expect("expected client message")
            .expect("client message read failed");
        websocket
            .send(message)
            .await
            .expect("websocket echo failed");
    });

    macro_client_echo()
        .await
        .expect("client websocket macro should receive the echoed frame");
    echo_server.await.expect("echo server task failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn service_registry_methods_cover_all_http_variants() {
    let services = load_registered_services();

    let get = service_by_name(&services, "method_get");
    assert_service_method(get, Method::GET, "/method/get", "GET", "get-ok").await;
    assert_service_rejects_method(get, Method::POST, "/method/get").await;

    let post = service_by_name(&services, "method_post");
    assert_service_method(post, Method::POST, "/method/post", "POST", "post-ok").await;
    assert_service_rejects_method(post, Method::GET, "/method/post").await;

    let put = service_by_name(&services, "method_put");
    assert_service_method(put, Method::PUT, "/method/put", "PUT", "put-ok").await;
    assert_service_rejects_method(put, Method::GET, "/method/put").await;

    let delete = service_by_name(&services, "method_delete");
    assert_service_method(
        delete,
        Method::DELETE,
        "/method/delete",
        "DELETE",
        "delete-ok",
    )
    .await;
    assert_service_rejects_method(delete, Method::GET, "/method/delete").await;

    let patch = service_by_name(&services, "method_patch");
    assert_service_method(patch, Method::PATCH, "/method/patch", "PATCH", "patch-ok").await;
    assert_service_rejects_method(patch, Method::GET, "/method/patch").await;

    let trace = service_by_name(&services, "method_trace");
    assert_service_method(trace, Method::TRACE, "/method/trace", "TRACE", "trace-ok").await;
    assert_service_rejects_method(trace, Method::GET, "/method/trace").await;

    let connect = service_by_name(&services, "method_connect");
    assert_service_method(
        connect,
        Method::CONNECT,
        "/method/connect",
        "CONNECT",
        "connect-ok",
    )
    .await;
    assert_service_rejects_method(connect, Method::GET, "/method/connect").await;

    let options = service_by_name(&services, "method_options");
    let options_response = invoke_service(options, Method::OPTIONS, "/method/options", None)
        .await
        .expect("options invocation failed");
    assert_eq!(options_response.status, 200);
    assert_eq!(header(&options_response, "x-method"), None);
    assert!(options_response.body.is_empty());
    assert_service_rejects_method(options, Method::GET, "/method/options").await;

    let head = service_by_name(&services, "method_head");
    let response = invoke_service(head, Method::HEAD, "/method/head", None)
        .await
        .expect("head invocation failed");
    assert_eq!(response.status, 200);
    assert_eq!(header(&response, "x-method"), Some("HEAD"));
    assert!(response.body.is_empty());
    assert_service_rejects_method(head, Method::GET, "/method/head").await;
}

#[test]
fn parse_http_response_parses_status_headers_and_body() {
    let raw = b"HTTP/1.1 200 OK\r\nX-Test: value\r\nContent-Length: 5\r\n\r\nhello";
    let parsed = parse_http_response(raw).expect("expected response parser to succeed");
    assert_eq!(parsed.status, 200);
    assert_eq!(header(&parsed, "x-test"), Some("value"));
    assert_eq!(parsed.body, b"hello");
}

#[test]
fn parse_http_response_rejects_missing_header_terminator() {
    let raw = b"HTTP/1.1 200 OK\r\nX-Test: value\r\n";
    let err = parse_http_response(raw).expect_err("expected parser error");
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

#[test]
fn find_bytes_handles_edge_cases() {
    assert_eq!(find_bytes(b"abc123", b""), Some(0));
    assert_eq!(find_bytes(b"abc123", b"123"), Some(3));
    assert_eq!(find_bytes(b"abc123", b"zzz"), None);
}

async fn assert_method(
    port: u16,
    method: &str,
    path: &str,
    expected_header: &str,
    expected_body: &str,
) {
    let response = raw_http_request(port, method, path, Some("payload"))
        .await
        .unwrap_or_else(|e| panic!("request failed for {method} {path}: {e}"));
    assert_eq!(
        response.status, 200,
        "unexpected status for {method} {path}"
    );
    assert_eq!(
        header(&response, "x-method"),
        Some(expected_header),
        "missing/invalid x-method for {method} {path}"
    );
    assert_eq!(
        String::from_utf8_lossy(&response.body),
        expected_body,
        "unexpected body for {method} {path}"
    );
}

fn load_registered_services() -> Vec<Service> {
    let mut registry = ServiceRegistry::default();
    let services: Vec<Service> = inventory::iter::<ServiceRegistration>
        .into_iter()
        .map(|reg| (reg.register)(&mut registry))
        .collect();
    registry.services.extend(services);
    registry.services
}

fn service_by_name<'a>(services: &'a [Service], name: &str) -> &'a Service {
    services
        .iter()
        .find(|service| service.name() == name)
        .unwrap_or_else(|| panic!("failed to locate registered service {name}"))
}

async fn assert_service_method(
    service: &Service,
    method: Method,
    path: &str,
    expected_header: &str,
    expected_body: &str,
) {
    let response = invoke_service(service, method, path, Some("payload"))
        .await
        .unwrap_or_else(|e| panic!("service invoke failed for {} {path}: {e:?}", service.name()));
    assert_eq!(
        response.status,
        200,
        "unexpected status for {}",
        service.name()
    );
    assert_eq!(
        header(&response, "x-method"),
        Some(expected_header),
        "unexpected x-method for {}",
        service.name()
    );
    assert_eq!(
        String::from_utf8_lossy(&response.body),
        expected_body,
        "unexpected response body for {}",
        service.name()
    );
}

async fn assert_service_rejects_method(service: &Service, method: Method, path: &str) {
    let request = build_request(service, method, path, None).expect("request build failed");
    let serves = service.serves(&request).await;
    assert!(
        !serves,
        "service {} unexpectedly accepted method {}",
        service.name(),
        request.method()
    );
}

async fn invoke_service(
    service: &Service,
    method: Method,
    path: &str,
    body: Option<&str>,
) -> Result<RawResponse, PortfuError> {
    let mut request = build_request(service, method, path, body)
        .map_err(|e| PortfuError::Internal(format!("request build error: {e}")))?;
    let serves = service.serves(&request).await;
    if !serves {
        return Err(PortfuError::Internal(format!(
            "service {} did not serve {} {}",
            service.name(),
            request.method(),
            request.uri().path()
        )));
    }
    let response = service.serve(&mut request).await?;
    let response: http::Response<_> = response.into();
    let status = response.status().as_u16();
    let mut headers = HashMap::new();
    for (k, v) in response.headers() {
        headers.insert(
            k.as_str().to_ascii_lowercase(),
            v.to_str().unwrap_or_default().to_string(),
        );
    }
    let bytes = BodyExt::collect(response.into_body())
        .await
        .map_err(|e| PortfuError::Internal(format!("failed to collect response body: {e:?}")))?
        .to_bytes()
        .to_vec();
    Ok(RawResponse {
        status,
        headers,
        body: bytes,
    })
}

fn build_request(
    service: &Service,
    method: Method,
    path: &str,
    body: Option<&str>,
) -> Result<Request, http::Error> {
    let bytes = hyper::body::Bytes::from(body.unwrap_or_default().to_string());
    let request = http::Request::builder()
        .method(method)
        .uri(path)
        .body(http_body_util::Full::new(bytes))?;
    Ok(Request::new(RequestType::Sized(request), service.route()))
}

fn header<'a>(response: &'a RawResponse, key: &str) -> Option<&'a str> {
    response
        .headers
        .get(&key.to_ascii_lowercase())
        .map(String::as_str)
}

async fn wait_for_server(port: u16) -> Result<(), std::io::Error> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match raw_http_request(port, "GET", "/method/get", None).await {
            Ok(response) if response.status == 200 => return Ok(()),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::ConnectionRefused => {}
            Err(e) => return Err(e),
        }
        if tokio::time::Instant::now() > deadline {
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "timed out waiting for server",
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn connect_with_retry(port: u16) -> Result<TcpStream, std::io::Error> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(stream) => return Ok(stream),
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {}
            Err(error) => return Err(error),
        }
        if tokio::time::Instant::now() > deadline {
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "timed out connecting to server",
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn raw_http_request(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<RawResponse, std::io::Error> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
    let body_text = body.unwrap_or_default();
    let body_bytes = body_text.as_bytes();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
        body_bytes.len(),
        body_text
    );
    stream.write_all(request.as_bytes()).await?;
    stream.shutdown().await?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    parse_http_response(&raw)
}

fn tls_client_config(
    server_certificate: &CertificateDer<'static>,
    identity: Option<&TestClientCertificate>,
) -> ClientConfig {
    let mut roots = RootCertStore::empty();
    roots
        .add(server_certificate.clone())
        .expect("server certificate should be a valid trust anchor");
    let builder = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 should be supported")
        .with_root_certificates(roots);
    match identity {
        Some(identity) => builder
            .with_client_auth_cert(
                vec![identity.certificate.clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.private_key.clone())),
            )
            .expect("client certificate and key should match"),
        None => builder.with_no_client_auth(),
    }
}

async fn tls_connect(
    port: u16,
    config: Arc<ClientConfig>,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, std::io::Error> {
    let stream = connect_with_retry(port).await?;
    tokio_rustls::TlsConnector::from(config)
        .connect(
            ServerName::try_from("localhost")
                .expect("localhost should be a valid server name")
                .to_owned(),
            stream,
        )
        .await
        .map_err(std::io::Error::other)
}

async fn tls_http_request(
    port: u16,
    config: Arc<ClientConfig>,
    path: &str,
) -> Result<RawResponse, std::io::Error> {
    let mut stream = tls_connect(port, config).await?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    parse_http_response(&raw)
}

async fn assert_tls_response(
    port: u16,
    config: Arc<ClientConfig>,
    path: &str,
    expected_status: u16,
    expected_body: &[u8],
) {
    let response = tls_http_request(port, config, path)
        .await
        .unwrap_or_else(|error| panic!("TLS request to {path} failed: {error}"));
    assert_eq!(
        response.status, expected_status,
        "unexpected status for {path}"
    );
    if !expected_body.is_empty() {
        assert_eq!(response.body, expected_body, "unexpected body for {path}");
    }
}

fn parse_http_response(raw: &[u8]) -> Result<RawResponse, std::io::Error> {
    let header_end = find_bytes(raw, b"\r\n\r\n").ok_or_else(|| {
        std::io::Error::new(
            ErrorKind::InvalidData,
            "failed to parse HTTP response headers",
        )
    })?;
    let head = std::str::from_utf8(&raw[..header_end]).map_err(|e| {
        std::io::Error::new(
            ErrorKind::InvalidData,
            format!("response headers were not utf8: {e}"),
        )
    })?;

    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidData, "missing status line"))?;
    let mut status_parts = status_line.split_whitespace();
    let _http_version = status_parts.next();
    let status = status_parts
        .next()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidData, "missing status code"))?
        .parse::<u16>()
        .map_err(|e| {
            std::io::Error::new(ErrorKind::InvalidData, format!("bad status code: {e}"))
        })?;

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    Ok(RawResponse {
        status,
        headers,
        body: raw[(header_end + 4)..].to_vec(),
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn reserve_port() -> Result<u16, std::io::Error> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}
