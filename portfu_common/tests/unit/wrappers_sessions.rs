use crate::router::route::Route;
use crate::server::builder::ServerBuilder;
use crate::service::request::{Request, RequestType};
use crate::wrappers::sessions::{SESSION_CLIENT_IDS, SessionManager};
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
    assert_eq!(
        crate::service::request::client_ip(&request).to_string(),
        "192.0.2.10"
    );

    let mut request = request_with_forwarded_ip();
    request.insert(peer);
    request.insert(Arc::new(
        ServerBuilder::new().trusted_proxy(peer.ip()).build(),
    ));
    assert_eq!(
        crate::service::request::client_ip(&request).to_string(),
        "203.0.113.7"
    );
}

#[test]
fn stale_client_session_index_is_removed() {
    let client_id = "stale-session-id";
    SESSION_CLIENT_IDS.insert(client_id.to_string(), "missing-server-id".to_string());
    assert!(SessionManager::get_session_from_id(client_id).is_none());
    assert!(!SESSION_CLIENT_IDS.contains_key(client_id));
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

use crate::router::middleware::Middleware;
use crate::service::response::Response;
use crate::wrappers::sessions::{MemorySessionStore, Session, SessionStore};
use std::time::Duration;
use tokio::sync::RwLock;

#[tokio::test]
async fn anonymous_sessions_are_local_until_written_and_preserve_handler_cookies() {
    let store = Arc::new(MemorySessionStore::new(2));
    let manager = SessionManager::new(Duration::from_secs(60), false).store(store.clone());
    let mut request = request_with_forwarded_ip();
    manager.before(&mut request).await.unwrap();
    let session = request.get::<Arc<RwLock<Session>>>().unwrap().clone();
    let id = session.read().await.id;
    let mut response = Response::ok("ok");
    response.headers_mut().insert(
        http::header::CACHE_CONTROL,
        "public, max-age=3600".parse().unwrap(),
    );
    manager
        .after_with_request(&request, &mut response)
        .await
        .unwrap();
    assert!(store.load(id).await.unwrap().is_none());
    assert!(response.headers().get(http::header::SET_COOKIE).is_none());
    assert_eq!(
        response.headers()[http::header::CACHE_CONTROL],
        "public, max-age=3600"
    );

    session
        .write()
        .await
        .data
        .insert("handler wrote data".to_string());
    response
        .headers_mut()
        .append(http::header::SET_COOKIE, "theme=dark".parse().unwrap());
    manager
        .after_with_request(&request, &mut response)
        .await
        .unwrap();
    assert!(store.load(id).await.unwrap().is_some());
    assert_eq!(
        response
            .headers()
            .get_all(http::header::SET_COOKIE)
            .iter()
            .count(),
        2
    );
    assert!(response.headers().get("session_id").is_none());
    assert_eq!(
        response.headers()[http::header::CACHE_CONTROL],
        "private, no-store"
    );
}

#[tokio::test]
async fn bypass_prevents_loading_persistence_and_cookies() {
    let store = Arc::new(MemorySessionStore::new(2));
    let manager = SessionManager::default()
        .store(store.clone())
        .skip_when(|request| request.headers().contains_key(http::header::AUTHORIZATION));
    let mut request = request_with_forwarded_ip();
    request.headers_mut().insert(
        http::header::AUTHORIZATION,
        "Bearer test-api-key".parse().unwrap(),
    );
    manager.before(&mut request).await.unwrap();
    assert!(request.get::<Arc<RwLock<Session>>>().is_none());
    // API authentication may supply a request-local session for compatible extractors.
    let local = Arc::new(RwLock::new(Session::default()));
    let id = local.read().await.id;
    local.write().await.data.insert("API principal".to_string());
    request.insert(local);
    let mut response = Response::ok("ok");
    manager
        .after_with_request(&request, &mut response)
        .await
        .unwrap();
    assert!(store.load(id).await.unwrap().is_none());
    assert!(response.headers().get(http::header::SET_COOKIE).is_none());

    let mut explicit = request_with_forwarded_ip();
    explicit.skip_session();
    SessionManager::default()
        .before(&mut explicit)
        .await
        .unwrap();
    assert!(explicit.get::<Arc<RwLock<Session>>>().is_none());
}

#[tokio::test]
async fn rotation_revokes_the_old_id_and_cannot_be_undone_by_an_inflight_request() {
    let store = Arc::new(MemorySessionStore::new(2));
    let manager = SessionManager::new(Duration::from_secs(60), false).store(store.clone());
    let mut request = request_with_forwarded_ip();
    manager.before(&mut request).await.unwrap();
    let old = request.get::<Arc<RwLock<Session>>>().unwrap().clone();
    let old_id = old.read().await.id;
    old.write().await.data.insert("pre-login data".to_string());
    manager
        .after_with_request(&request, &mut Response::ok("ok"))
        .await
        .unwrap();
    SessionManager::rotate_session(&mut request).await.unwrap();
    assert!(store.load(old_id).await.unwrap().is_none());
    let new = request.get::<Arc<RwLock<Session>>>().unwrap().clone();
    let new_id = new.read().await.id;
    assert_ne!(new_id, old_id);
    assert_eq!(
        new.read().await.data.get::<String>().unwrap(),
        "pre-login data"
    );
    assert!(old.read().await.data.is_empty());
    new.write().await.data.insert(42_u64);
    let mut response = Response::ok("ok");
    manager
        .after_with_request(&request, &mut response)
        .await
        .unwrap();
    assert!(
        response.headers()[http::header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains(&new_id.to_string())
    );
    store.save(old, Duration::from_secs(60)).await.unwrap();
    assert!(store.load(old_id).await.unwrap().is_none());
    assert_eq!(
        *store
            .load(new_id)
            .await
            .unwrap()
            .unwrap()
            .read()
            .await
            .data
            .get::<u64>()
            .unwrap(),
        42
    );
}

#[tokio::test]
async fn session_capacity_is_hard_bounded_and_expiration_frees_space() {
    let store = MemorySessionStore::new(1);
    let first = Arc::new(RwLock::new(Session::default()));
    store
        .save(first.clone(), Duration::from_secs(60))
        .await
        .unwrap();
    let second = Arc::new(RwLock::new(Session::default()));
    assert!(
        store
            .save(second.clone(), Duration::from_secs(60))
            .await
            .is_err()
    );
    first.write().await.last_update = std::time::Instant::now() - Duration::from_secs(120);
    store.cleanup().await.unwrap();
    store
        .save(second.clone(), Duration::from_secs(60))
        .await
        .unwrap();
    let id = second.read().await.id;
    assert!(store.load(id).await.unwrap().is_some());
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn oauth_reuses_existing_manager_and_preserves_its_cookie_settings() {
    let server = ServerBuilder::new()
        .enable_sessions()
        .secure(false)
        .finish_sessions()
        .enable_oauth(crate::auth::oauth::OAUTH::CUSTOM)
        .finish_oauth()
        .build();
    assert_eq!(
        server
            .middleware
            .iter()
            .filter(|m| m.is_session_manager())
            .count(),
        1
    );
    let manager = server
        .middleware
        .iter()
        .find(|m| m.is_session_manager())
        .unwrap();
    let mut request = request_with_forwarded_ip();
    manager.before(&mut request).await.unwrap();
    request
        .get::<Arc<RwLock<Session>>>()
        .unwrap()
        .write()
        .await
        .data
        .insert(42_u64);
    let mut response = Response::ok("ok");
    manager
        .after_with_request(&request, &mut response)
        .await
        .unwrap();
    assert!(
        !response.headers()[http::header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Secure")
    );
    let reverse = ServerBuilder::new()
        .enable_oauth(crate::auth::oauth::OAUTH::CUSTOM)
        .finish_oauth()
        .enable_sessions()
        .finish_sessions()
        .build();
    assert_eq!(
        reverse
            .middleware
            .iter()
            .filter(|m| m.is_session_manager())
            .count(),
        1
    );
}

#[tokio::test]
async fn configured_backend_is_used_by_auth_filters_before_middleware_and_across_managers() {
    use crate::wrappers::sessions::get_session_from_request;
    let store = Arc::new(MemorySessionStore::new(2));
    let session = Arc::new(RwLock::new(Session::default()));
    let id = session.read().await.id;
    session
        .write()
        .await
        .data
        .insert("shared principal".to_string());
    store.save(session, Duration::from_secs(60)).await.unwrap();
    let server = Arc::new(
        ServerBuilder::new()
            .session_manager(SessionManager::default().store(store.clone()))
            .build(),
    );
    let mut request = request_with_forwarded_ip();
    request.headers_mut().insert(
        http::header::COOKIE,
        format!("session_id={id}").parse().unwrap(),
    );
    request.insert(server);
    request.insert(SocketAddr::from(([192, 0, 2, 99], 8080)));
    let found = get_session_from_request(&request).await.unwrap();
    assert_eq!(
        found.read().await.data.get::<String>().unwrap(),
        "shared principal"
    );
    let other = SessionManager::default().store(store);
    other.before(&mut request).await.unwrap();
    assert_eq!(
        request
            .get::<Arc<RwLock<Session>>>()
            .unwrap()
            .read()
            .await
            .id,
        id
    );
    let mut response = Response::ok("ok");
    other
        .after_with_request(&request, &mut response)
        .await
        .unwrap();
    assert!(response.headers().get(http::header::SET_COOKIE).is_none());
    assert_eq!(
        response.headers()[http::header::CACHE_CONTROL],
        "private, no-store"
    );
}
