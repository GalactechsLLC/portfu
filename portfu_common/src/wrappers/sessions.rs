use crate::error::PortfuError;
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::server::Server;
use crate::server::builder::ServerBuilder;
use crate::service::request::{FromRequest, Request};
use crate::service::response::Response;
use cookie::Cookie;
use dashmap::DashMap;
use http::header;
use http::{Extensions, HeaderName, HeaderValue};
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

pub static SESSION_HEADER: &str = "session_id";
pub static SESSIONS: Lazy<Arc<DashMap<String, Arc<RwLock<Session>>>>> = Lazy::new(Default::default);
pub static SESSION_CLIENT_IDS: Lazy<Arc<DashMap<String, String>>> = Lazy::new(Default::default);

#[derive(Clone)]
struct PendingSessionCookie(HeaderValue);

pub struct Session {
    pub data: Extensions,
    pub last_update: Instant,
    pub id: Uuid,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            data: Extensions::new(),
            last_update: Instant::now(),
            id: Uuid::new_v4(),
        }
    }
}

#[derive(Clone)]
pub struct SessionState(pub Arc<RwLock<Session>>);

impl SessionState {
    pub fn inner(&self) -> Arc<RwLock<Session>> {
        self.0.clone()
    }
}

impl FromRequest<Request> for SessionState {
    type Error = PortfuError;

    fn try_from<'a>(
        value: &'a mut Request,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            value
                .get::<Arc<RwLock<Session>>>()
                .cloned()
                .map(SessionState)
                .ok_or_else(|| {
                    PortfuError::Unauthorized(
                        "Failed to find active session on request".to_string(),
                    )
                })
        })
    }
}

pub struct SessionManager {
    pub session_duration: Duration,
    pub secure: bool,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self {
            session_duration: Duration::from_secs(60 * 30), //30 minutes
            secure: true,
        }
    }
}

impl SessionManager {
    pub fn new(session_duration: Duration, secure: bool) -> Self {
        Self {
            session_duration,
            secure,
        }
    }

    async fn create_session_cookie(
        &self,
        request: &Request,
    ) -> (Cookie<'static>, Arc<RwLock<Session>>) {
        let salt = request_best_guess_ip(request);
        let client_session_id = Uuid::new_v4();
        let mut hasher = Sha256::new();
        hasher.update([client_session_id.to_string().as_bytes(), salt.as_bytes()].concat());
        let server_session_id = hex::encode(hasher.finalize());
        let cookie = Cookie::build((SESSION_HEADER, client_session_id.to_string()))
            .path("/")
            .secure(self.secure)
            .http_only(true)
            .same_site(cookie::SameSite::Lax)
            .build();
        let session = Arc::new(RwLock::new(Session {
            id: client_session_id,
            ..Default::default()
        }));
        SESSIONS.insert(server_session_id.clone(), session.clone());
        SESSION_CLIENT_IDS.insert(client_session_id.to_string(), server_session_id);
        (cookie.into_owned(), session)
    }

    pub async fn get_session(
        &self,
        request: &Request,
        session_cookie: Cookie<'_>,
    ) -> Option<Arc<RwLock<Session>>> {
        let salt = request_best_guess_ip(request);
        let mut hasher = Sha256::new();
        hasher.update([session_cookie.value_trimmed().as_bytes(), salt.as_bytes()].concat());
        let server_session_id = hex::encode(hasher.finalize());
        if let Some(session) = SESSIONS.get(&server_session_id).map(|v| v.value().clone()) {
            if Instant::now().duration_since(session.read().await.last_update)
                >= self.session_duration
            {
                SESSIONS.remove(&server_session_id);
                SESSION_CLIENT_IDS.remove(session_cookie.value_trimmed());
                None
            } else {
                session.write().await.last_update = Instant::now();
                Some(session)
            }
        } else {
            None
        }
    }

    pub fn get_session_from_id(client_session_id: &str) -> Option<Arc<RwLock<Session>>> {
        let server_session_id = SESSION_CLIENT_IDS
            .get(client_session_id)
            .map(|entry| entry.value().clone())?;
        match SESSIONS.get(&server_session_id) {
            Some(session) => Some(session.value().clone()),
            None => {
                SESSION_CLIENT_IDS.remove(client_session_id);
                None
            }
        }
    }

    pub async fn cleanup_expired(&self) {
        let mut to_remove = vec![];
        for entry in SESSIONS.iter() {
            let session = entry.value().read().await;
            if session.last_update.elapsed() > self.session_duration {
                to_remove.push((entry.key().clone(), session.id.to_string()));
            }
        }
        for (key, client_id) in to_remove {
            SESSIONS.remove(&key);
            SESSION_CLIENT_IDS.remove(&client_id);
        }
    }
}

pub struct SessionServerBuilder {
    builder: ServerBuilder,
    manager: SessionManager,
}

impl SessionServerBuilder {
    pub fn session_duration(mut self, duration: Duration) -> Self {
        self.manager.session_duration = duration;
        self
    }

    pub fn duration(self, duration: Duration) -> Self {
        self.session_duration(duration)
    }

    pub fn secure(mut self, secure: bool) -> Self {
        self.manager.secure = secure;
        self
    }

    pub fn session_manager(mut self, manager: SessionManager) -> Self {
        self.manager = manager;
        self
    }

    pub fn finish_sessions(self) -> ServerBuilder {
        self.builder.wrap(Arc::new(self.manager))
    }

    pub fn build(self) -> crate::server::Server {
        self.finish_sessions().build()
    }
}

impl ServerBuilder {
    pub fn enable_sessions(self) -> SessionServerBuilder {
        SessionServerBuilder {
            builder: self,
            manager: SessionManager::default(),
        }
    }

    pub fn session_manager(self, manager: SessionManager) -> Self {
        self.wrap(Arc::new(manager))
    }
}

fn request_best_guess_ip(request: &Request) -> String {
    let trust_proxy_headers = request
        .get::<Arc<Server>>()
        .is_some_and(|server| server.config.trust_proxy_headers);
    if trust_proxy_headers {
        if let Some(real_ip) = request.headers().get("x-real-ip")
            && let Ok(as_str) = real_ip.to_str()
        {
            return as_str.to_string();
        }
        if let Some(cloudflare_ip) = request.headers().get("cf-connecting-ip")
            && let Ok(as_str) = cloudflare_ip.to_str()
        {
            return as_str.to_string();
        }
    }
    request
        .get::<SocketAddr>()
        .map(|s| s.ip().to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

#[cfg(test)]
#[path = "../../tests/unit/wrappers_sessions.rs"]
mod tests;

pub fn get_session_cookie_from_request(request: &Request) -> Option<Cookie<'_>> {
    let mut session_cookie = None;
    'outer: for value in request.headers().get_all(header::COOKIE) {
        match value.to_str() {
            Ok(val) => {
                let mut split_cookies = Cookie::split_parse(val);
                while let Some(Ok(cookie)) = split_cookies.next() {
                    if cookie.name() == SESSION_HEADER {
                        session_cookie = Some(cookie);
                        break 'outer;
                    }
                }
            }
            Err(_) => continue,
        }
    }
    session_cookie
}

pub async fn get_session_from_request(request: &Request) -> Option<Arc<RwLock<Session>>> {
    let cookie = get_session_cookie_from_request(request)?;
    let salt = request_best_guess_ip(request);
    let mut hasher = Sha256::new();
    hasher.update([cookie.value_trimmed().as_bytes(), salt.as_bytes()].concat());
    let server_session_id = hex::encode(hasher.finalize());
    SESSIONS.get(&server_session_id).map(|v| v.value().clone())
}

impl Middleware for SessionManager {
    fn name(&self) -> &str {
        "SessionManager"
    }

    fn before<'a>(
        &'a self,
        request: &'a mut Request,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>,
    > {
        Box::pin(async move {
            self.cleanup_expired().await;
            let session = match get_session_cookie_from_request(request) {
                None => {
                    let (cookie, session) = self.create_session_cookie(request).await;
                    if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
                        request.insert(PendingSessionCookie(value));
                    }
                    session
                }
                Some(cookie) => {
                    if let Some(session) = self.get_session(request, cookie).await {
                        session
                    } else {
                        let (cookie, session) = self.create_session_cookie(request).await;
                        if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
                            request.insert(PendingSessionCookie(value));
                        }
                        session
                    }
                }
            };
            request.insert(session);
            Ok(MiddlewareResult::Continue)
        })
    }

    fn after_with_request<'a>(
        &'a self,
        request: &'a Request,
        response: &'a mut Response,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>,
    > {
        Box::pin(async move {
            if let Some(value) = request.get::<PendingSessionCookie>() {
                response
                    .headers_mut()
                    .insert(HeaderName::from_static("set-cookie"), value.0.clone());
                response
                    .headers_mut()
                    .insert(HeaderName::from_static(SESSION_HEADER), value.0.clone());
            }
            Ok(MiddlewareResult::Continue)
        })
    }

    fn after<'a>(
        &'a self,
        _: &'a mut Response,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>,
    > {
        Box::pin(async move { Ok(MiddlewareResult::Continue) })
    }
}
