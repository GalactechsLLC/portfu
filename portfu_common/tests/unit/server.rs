use super::{DEFAULT_SCOPE, Route, Server};
use crate::server::builder::ServerBuilder;
use crate::server::connection::ConnectionInfo;
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
