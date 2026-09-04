use super::best_guess_public_ip;
use crate::router::route::Route;
use crate::server::builder::ServerBuilder;
use crate::service::request::{Request, RequestType};
use http_body_util::Full;
use hyper::body::Bytes;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

#[test]
fn rate_limit_identity_ignores_untrusted_forwarding_headers() {
    let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 20)), 8080);
    let raw = http::Request::builder()
        .uri("/")
        .header("cf-connecting-ip", "203.0.113.8")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let mut request = Request::new(
        RequestType::Sized(raw),
        Arc::new(Route::new("/".to_string())),
    );
    request.insert(peer);
    request.insert(Arc::new(ServerBuilder::new().build()));

    assert_eq!(best_guess_public_ip(&request), "192.0.2.20");
}
