use crate::router::route::Route;
use crate::server::builder::ServerBuilder;
use crate::service::request::{Request, RequestType};
use crate::wrappers::rate_limits::best_guess_public_ip;
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

use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::wrappers::rate_limits::{RateLimit, RateLimiter};
use std::time::Duration;

fn request(path: &str, peer: u8) -> Request {
    let raw = http::Request::builder()
        .uri(path)
        .body(Full::new(Bytes::new()))
        .unwrap();
    let mut request = Request::new(
        RequestType::Sized(raw),
        Arc::new(Route::new("/items/{id}".into())),
    );
    request.insert(SocketAddr::from(([192, 0, 2, peer], 8080)));
    request
}

#[tokio::test]
async fn requests_per_window_is_not_multiplied_and_global_count_spans_paths() {
    let limiter = RateLimiter::with_global_limit(RateLimit::new(120, 60, 0));
    for id in 0..120 {
        assert!(matches!(
            limiter
                .before(&mut request(&format!("/items/{id}"), 1))
                .await
                .unwrap(),
            MiddlewareResult::Continue
        ));
    }
    assert!(matches!(
        limiter.before(&mut request("/items/new", 1)).await.unwrap(),
        MiddlewareResult::Return(_)
    ));
    assert!(matches!(
        limiter.before(&mut request("/items/new", 2)).await.unwrap(),
        MiddlewareResult::Continue
    ));
}

#[tokio::test]
async fn template_limits_group_ids_and_concurrent_requests_cannot_bypass_limits() {
    let limiter = Arc::new(RateLimiter::with_global_limit(RateLimit::new(100, 60, 0)));
    limiter
        .set_path_limit("/items/{id}", RateLimit::new(5, 60, 0))
        .await;
    let mut tasks = Vec::new();
    for id in 0..50 {
        let limiter = limiter.clone();
        tasks.push(tokio::spawn(async move {
            matches!(
                limiter
                    .before(&mut request(&format!("/items/{id}"), 1))
                    .await
                    .unwrap(),
                MiddlewareResult::Continue
            )
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        accepted += usize::from(task.await.unwrap());
    }
    assert_eq!(accepted, 5);
}

#[tokio::test]
async fn clients_are_bounded_and_expired_entries_are_evicted() {
    let limiter = RateLimiter::with_global_limit(RateLimit::new(1, 0, 0))
        .capacity(1, 2)
        .client_idle_timeout(Duration::ZERO);
    limiter.before(&mut request("/items/1", 1)).await.unwrap();
    limiter.cleanup_expired().await;
    assert!(matches!(
        limiter.before(&mut request("/items/2", 2)).await.unwrap(),
        MiddlewareResult::Continue
    ));
    assert_eq!(limiter.client_rates.read().await.len(), 1);
    let bounded = RateLimiter::default().capacity(1, 2);
    bounded.before(&mut request("/items/1", 1)).await.unwrap();
    assert!(matches!(
        bounded.before(&mut request("/items/2", 2)).await.unwrap(),
        MiddlewareResult::Return(_)
    ));
    assert_eq!(bounded.client_rates.read().await.len(), 1);
}

#[test]
fn proxy_identity_requires_a_trusted_peer_and_a_valid_ip() {
    let mut request = request("/items/1", 1);
    request
        .headers_mut()
        .insert("x-real-ip", "not-an-ip".parse().unwrap());
    request.insert(Arc::new(
        ServerBuilder::new()
            .trusted_proxy("192.0.2.1".parse().unwrap())
            .build(),
    ));
    assert_eq!(best_guess_public_ip(&request), "192.0.2.1");
    request
        .headers_mut()
        .insert("x-real-ip", "203.0.113.8".parse().unwrap());
    assert_eq!(best_guess_public_ip(&request), "203.0.113.8");
    request.insert(SocketAddr::from(([192, 0, 2, 2], 8080)));
    assert_eq!(best_guess_public_ip(&request), "192.0.2.2");
}

#[test]
fn unused_forwarding_headers_are_never_a_fallback_identity() {
    let mut request = request("/items/1", 1);
    request
        .headers_mut()
        .insert("cf-connecting-ip", "203.0.113.8".parse().unwrap());
    request.insert(Arc::new(
        ServerBuilder::new()
            .trusted_proxy("192.0.2.1".parse().unwrap())
            .build(),
    ));
    assert_eq!(best_guess_public_ip(&request), "192.0.2.1");
    request.insert(Arc::new(
        ServerBuilder::new()
            .trusted_proxy("192.0.2.1".parse().unwrap())
            .forwarded_ip_header(http::HeaderName::from_static("cf-connecting-ip"))
            .build(),
    ));
    assert_eq!(best_guess_public_ip(&request), "203.0.113.8");
}
