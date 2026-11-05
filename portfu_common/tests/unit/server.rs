use crate::router::route::Route;
use crate::server::builder::ServerBuilder;
use crate::server::connection::ConnectionInfo;
use crate::server::{DEFAULT_SCOPE, Server};
use crate::service::request::{Request, RequestType};
use http::Extensions;
use http_body_util::Full;
use hyper::body::Bytes;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;

struct DefaultState;
struct TenantState;
struct PreviousState;

#[test]
fn server_handle_requests_shutdown() {
    let server = ServerBuilder::new().build();
    let handle = server.handle();

    handle.shutdown();

    assert!(!server.run.load(Ordering::Relaxed));
    assert!(*server.shutdown.borrow());
}

#[test]
fn set_request_scope_state_replaces_previous_candidate_scope_state() {
    let mut scoped_state = HashMap::new();
    let mut default_extensions = Extensions::new();
    default_extensions.insert(Arc::new(DefaultState));
    scoped_state.insert(DEFAULT_SCOPE.to_string(), default_extensions);

    let mut tenant_extensions = Extensions::new();
    tenant_extensions.insert(Arc::new(TenantState));
    scoped_state.insert("tenant".to_string(), tenant_extensions);

    let request = http::Request::builder()
        .uri("/scoped")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    let mut request = Request::new(
        RequestType::Sized(request),
        Arc::new(Route::new("/scoped".to_string())),
    );
    request.insert(Arc::new(PreviousState));

    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);
    let connection = ConnectionInfo::plaintext(address, address);
    Server::set_request_scope_state(&mut request, &scoped_state, "tenant", &connection);
    assert!(request.get::<Arc<PreviousState>>().is_none());
    assert!(request.get::<Arc<DefaultState>>().is_some());
    assert!(request.get::<Arc<TenantState>>().is_some());

    Server::set_request_scope_state(&mut request, &scoped_state, "other", &connection);
    assert!(request.get::<Arc<DefaultState>>().is_some());
    assert!(request.get::<Arc<TenantState>>().is_none());
}

#[test]
fn set_request_scope_state_preserves_pending_http_upgrade() {
    let mut scoped_state = HashMap::new();
    let mut default_extensions = Extensions::new();
    default_extensions.insert(Arc::new(DefaultState));
    scoped_state.insert(DEFAULT_SCOPE.to_string(), default_extensions);

    let upgrade = hyper::upgrade::on(http::Request::new(()));
    let request = http::Request::builder()
        .uri("/ws")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    let mut request = Request::new(
        RequestType::Sized(request),
        Arc::new(Route::new("/ws".to_string())),
    );
    request.insert(upgrade);

    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);
    let connection = ConnectionInfo::plaintext(address, address);
    Server::set_request_scope_state(&mut request, &scoped_state, DEFAULT_SCOPE, &connection);

    assert!(request.get::<hyper::upgrade::OnUpgrade>().is_some());
    assert!(request.get::<Arc<DefaultState>>().is_some());
}

#[tokio::test]
async fn header_timeout_closes_incomplete_request_and_is_a_warning() {
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::{TokioIo, TokioTimer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut client, server) = tokio::io::duplex(1024);
    let task = tokio::spawn(async move {
        http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(std::time::Duration::from_millis(50))
            .serve_connection(
                TokioIo::new(server),
                service_fn(|_| async {
                    Ok::<_, std::convert::Infallible>(http::Response::new(Full::new(Bytes::new())))
                }),
            )
            .await
            .expect_err("incomplete headers must time out")
    });
    client.write_all(b"GET / HTTP/1.1\r\nHost:").await.unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(error.is_timeout());
    assert_eq!(
        crate::server::connection_error_level(&error),
        log::Level::Warn
    );
    // The timeout still tears down the transport; this is not a disabled deadline.
    let mut response = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        client.read_to_end(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
}
