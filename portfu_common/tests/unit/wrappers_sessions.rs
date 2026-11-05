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

struct FailingSessionStore {
    inner: MemorySessionStore,
    fail: std::sync::atomic::AtomicBool,
}

impl SessionStore for FailingSessionStore {
    fn load<'a>(
        &'a self,
        id: uuid::Uuid,
    ) -> crate::wrappers::sessions::StoreFuture<'a, Option<Arc<RwLock<Session>>>> {
        self.inner.load(id)
    }

    fn save<'a>(
        &'a self,
        session: Arc<RwLock<Session>>,
        ttl: Duration,
    ) -> crate::wrappers::sessions::StoreFuture<'a, ()> {
        Box::pin(async move {
            if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(crate::error::PortfuError::ServiceUnavailable(
                    "backend unavailable".into(),
                ));
            }
            self.inner.save(session, ttl).await
        })
    }

    fn remove<'a>(&'a self, id: uuid::Uuid) -> crate::wrappers::sessions::StoreFuture<'a, ()> {
        self.inner.remove(id)
    }
}

#[tokio::test]
async fn read_only_session_does_not_save_after_handler_side_effects() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let store = Arc::new(FailingSessionStore {
        inner: MemorySessionStore::new(2),
        fail: AtomicBool::new(false),
    });
    let session = Arc::new(RwLock::new(Session::default()));
    session.write().await.data.insert(42_u64);
    let id = session.read().await.id;
    store
        .save(session.clone(), Duration::from_secs(60))
        .await
        .unwrap();
    let manager = SessionManager::default().store(store.clone());
    let mut request = request_with_forwarded_ip();
    request.headers_mut().insert(
        http::header::COOKIE,
        format!("session_id={id}").parse().unwrap(),
    );
    manager.before(&mut request).await.unwrap();
    // An upload reads its principal, commits its database write, then the backend fails.
    assert_eq!(session.read().await.data.get::<u64>(), Some(&42));
    let mut response = Response::json(serde_json::json!({"upload_id": 7}));
    store.fail.store(true, Ordering::Relaxed);
    manager
        .after_with_request(&request, &mut response)
        .await
        .unwrap();
    assert_eq!(response.status(), http::StatusCode::OK);
    assert_eq!(
        response.headers()[http::header::CACHE_CONTROL],
        "private, no-store"
    );
    assert!(!response.headers().contains_key(http::header::SET_COOKIE));

    // An outage already present at admission must stop the handler.
    let mut next = request_with_forwarded_ip();
    next.headers_mut().insert(
        http::header::COOKIE,
        format!("session_id={id}").parse().unwrap(),
    );
    assert!(manager.before(&mut next).await.is_err());

    // Authentication/logout changes still require durable persistence.
    session.write().await.data.remove::<u64>();
    assert!(
        manager
            .after_with_request(&request, &mut response)
            .await
            .is_err()
    );
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn abandoned_oauth_logins_cannot_exhaust_session_capacity_or_evict_application_data() {
    use crate::auth::oauth::OAUTH;
    // Exercise the real login route with a small capacity; the admission rule is
    // independent of the default 10,000-entry limit.
    let store = Arc::new(MemorySessionStore::new(8));
    let manager = SessionManager::default().store(store.clone());
    let established = Arc::new(RwLock::new(Session::default()));
    established
        .write()
        .await
        .data
        .insert("application principal".to_string());
    let established_id = established.read().await.id;
    store
        .save(established.clone(), Duration::from_secs(60))
        .await
        .unwrap();
    let server = ServerBuilder::new()
        .session_manager(manager.clone())
        .enable_oauth(OAUTH::KEYCLOAK)
        .client_id("test-client")
        .client_secret("test-secret")
        .auth_url("https://provider.example/authorize")
        .token_url("https://provider.example/token")
        .redirect_url("https://app.example/callback")
        .login_path("/keycloak/login")
        .build();
    let login = server
        .services
        .iter()
        .find(|service| service.name() == "oauth_login")
        .unwrap();
    let mut first = None;
    let mut latest = None;
    for _ in 0..20 {
        let mut request = request_with_forwarded_ip();
        manager.before(&mut request).await.unwrap();
        let session = request.get::<Arc<RwLock<Session>>>().unwrap().clone();
        let mut response = login.serve(&mut request).await.unwrap();
        manager
            .after_with_request(&request, &mut response)
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::FOUND);
        assert!(response.headers().contains_key(http::header::SET_COOKIE));
        first.get_or_insert((session.read().await.id, session.clone()));
        latest = Some(session.read().await.id);
    }
    let (first_id, first) = first.unwrap();
    assert!(store.load(first_id).await.unwrap().is_none());
    // An in-flight request must not resurrect an evicted handshake.
    store.save(first, Duration::from_secs(60)).await.unwrap();
    assert!(store.load(first_id).await.unwrap().is_none());
    assert!(store.load(latest.unwrap()).await.unwrap().is_some());
    assert!(store.load(established_id).await.unwrap().is_some());
    assert_eq!(
        established.read().await.data.get::<String>().unwrap(),
        "application principal"
    );
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn default_sized_store_remains_bounded_after_ten_thousand_pending_logins() {
    use crate::auth::oauth::{SessionCsrfToken, SessionPkceVerifier};
    let store = MemorySessionStore::new(10_000);
    let mut ids = Vec::new();
    for _ in 0..10_005 {
        let mut session = Session::default();
        session.data.insert(SessionCsrfToken("state".into()));
        session.data.insert(SessionPkceVerifier("verifier".into()));
        ids.push(session.id);
        store
            .save(Arc::new(RwLock::new(session)), Duration::from_secs(60))
            .await
            .unwrap();
    }
    for (index, id) in ids.into_iter().enumerate() {
        assert_eq!(store.load(id).await.unwrap().is_some(), index >= 5);
    }
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn pending_session_with_new_application_data_is_protected_before_its_next_save() {
    use crate::auth::oauth::{SessionCsrfToken, SessionPkceVerifier};
    let store = MemorySessionStore::new(1);
    let session = Arc::new(RwLock::new(Session::default()));
    {
        let mut session = session.write().await;
        session.data.insert(SessionCsrfToken("state".into()));
        session.data.insert(SessionPkceVerifier("verifier".into()));
    }
    let id = session.read().await.id;
    store
        .save(session.clone(), Duration::from_secs(60))
        .await
        .unwrap();
    session
        .write()
        .await
        .data
        .insert("DID principal".to_string());
    assert!(
        store
            .save(
                Arc::new(RwLock::new(Session::default())),
                Duration::from_secs(60)
            )
            .await
            .is_err()
    );
    assert!(store.load(id).await.unwrap().is_some());
}

#[tokio::test]
async fn session_data_replacements_and_mutable_borrows_require_persistence() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let store = Arc::new(FailingSessionStore {
        inner: MemorySessionStore::new(1),
        fail: AtomicBool::new(false),
    });
    let session = Arc::new(RwLock::new(Session::default()));
    let id = session.read().await.id;
    session.write().await.data.insert(42_u64);
    store
        .save(session.clone(), Duration::from_secs(60))
        .await
        .unwrap();
    let manager = SessionManager::default().store(store.clone());
    for replace in [false, true] {
        store.fail.store(false, Ordering::Relaxed);
        let mut request = request_with_forwarded_ip();
        request.headers_mut().insert(
            http::header::COOKIE,
            format!("session_id={id}").parse().unwrap(),
        );
        manager.before(&mut request).await.unwrap();
        if replace {
            session.write().await.data = Default::default();
        } else {
            *session.write().await.data.get_mut::<u64>().unwrap() = 7;
        }
        store.fail.store(true, Ordering::Relaxed);
        assert!(
            manager
                .after_with_request(&request, &mut Response::ok("ok"))
                .await
                .is_err()
        );
    }
}

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
