use super::{SESSION_CLIENT_IDS, SESSIONS, request_best_guess_ip};
use crate::router::route::Route;
use crate::server::builder::ServerBuilder;
use crate::service::request::{Request, RequestType};
use http_body_util::Full;
use hyper::body::Bytes;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

#[test]
fn forwarded_ip_headers_require_explicit_proxy_trust() {
    let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), 8080);
    let mut request = request_with_forwarded_ip();
    request.insert(peer);
    request.insert(Arc::new(ServerBuilder::new().build()));
    assert_eq!(request_best_guess_ip(&request), "192.0.2.10");

    let mut request = request_with_forwarded_ip();
    request.insert(peer);
    request.insert(Arc::new(
        ServerBuilder::new().trust_proxy_headers(true).build(),
    ));
    assert_eq!(request_best_guess_ip(&request), "203.0.113.7");
}

#[test]
fn stale_client_session_index_is_removed() {
    let client_id = "stale-session-id";
    SESSION_CLIENT_IDS.insert(client_id.to_string(), "missing-server-id".to_string());
    assert!(super::SessionManager::get_session_from_id(client_id).is_none());
    assert!(!SESSION_CLIENT_IDS.contains_key(client_id));
    SESSIONS.clear();
}

fn request_with_forwarded_ip() -> Request {
    let raw = http::Request::builder()
        .uri("/")
        .header("x-real-ip", "203.0.113.7")
        .body(Full::new(Bytes::new()))
        .unwrap();
    Request::new(
        RequestType::Sized(raw),
        Arc::new(Route::new("/".to_string())),
    )
}
