use futures_util::{SinkExt, StreamExt};
use http::Method;
use http_body_util::BodyExt;
use portfu::prelude::*;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpListener;
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

#[websocket("/ws/echo")]
async fn ws_echo(websocket: WebSocket) -> Result<(), PortfuError> {
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

struct TestServer {
    port: u16,
    stop_tx: Option<oneshot::Sender<()>>,
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
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
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
                let server_task = tokio::spawn(async move { server.run().await });
                let _ = stop_rx.await;
                server_task.abort();
                let _ = server_task.await;
            });
        });

        wait_for_server(port)
            .await
            .expect("server failed to become ready");

        Some(Self {
            port,
            stop_tx: Some(stop_tx),
            thread: Some(thread),
        })
    }

    async fn stop(mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            tokio::task::spawn_blocking(move || {
                let _ = thread.join();
            })
            .await
            .expect("failed joining server thread");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_methods_and_websocket_work_end_to_end() {
    let Some(server) = TestServer::start().await else {
        return;
    };
    let port = server.port;

    let health = raw_http_request(port, "GET", "/health", None)
        .await
        .expect("health request failed");
    assert_eq!(health.status, 200);
    assert_eq!(String::from_utf8_lossy(&health.body), "OK");

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
        match raw_http_request(port, "GET", "/health", None).await {
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
