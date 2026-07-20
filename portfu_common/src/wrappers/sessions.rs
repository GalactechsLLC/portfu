use crate::error::PortfuError;
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::service::request::Request;
use crate::service::response::Response;
use cookie::Cookie;
use dashmap::DashMap;
use http::header;
use http::{Extensions, HeaderName, HeaderValue};
use once_cell::sync::Lazy;
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
    async fn create_session_cookie(&self) -> (Cookie<'static>, Arc<RwLock<Session>>) {
        let client_session_id = Uuid::new_v4();
        let server_session_id = client_session_id.to_string();
        let cookie = Cookie::build((SESSION_HEADER, client_session_id.to_string()))
            .path("/")
            .secure(self.secure)
            .http_only(true)
            .same_site(cookie::SameSite::Lax)
            .build();
        let session = Arc::new(RwLock::new(Session {
            data: Extensions::new(),
            last_update: Instant::now(),
            id: client_session_id,
        }));
        SESSIONS.insert(server_session_id.clone(), session.clone());
        SESSION_CLIENT_IDS.insert(client_session_id.to_string(), server_session_id);
        (cookie.into_owned(), session)
    }

    pub async fn get_session(&self, session_cookie: Cookie<'_>) -> Option<Arc<RwLock<Session>>> {
        let server_session_id = SESSION_CLIENT_IDS
            .get(session_cookie.value_trimmed())
            .map(|value| value.value().clone())
            .unwrap_or_else(|| session_cookie.value_trimmed().to_string());
        if let Some(session) = SESSIONS.get(&server_session_id).map(|v| v.value().clone()) {
            if Instant::now().duration_since(session.read().await.last_update)
                >= self.session_duration
            {
                SESSIONS.remove(&server_session_id);
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
        let server_session_id = SESSION_CLIENT_IDS.get(client_session_id)?;
        SESSIONS
            .get(server_session_id.value())
            .map(|v| v.value().clone())
    }

    pub async fn cleanup_expired(&self) {
        let mut to_remove = vec![];
        for entry in SESSIONS.iter() {
            let session = entry.value().read().await;
            if session.last_update.elapsed() > self.session_duration {
                to_remove.push(entry.key().clone());
            }
        }
        for key in to_remove {
            SESSIONS.remove(&key);
        }
    }
}

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
    SessionManager::get_session_from_id(cookie.value_trimmed())
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
                    let (cookie, session) = self.create_session_cookie().await;
                    if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
                        request.insert(PendingSessionCookie(value));
                    }
                    session
                }
                Some(cookie) => {
                    if let Some(session) = self.get_session(cookie).await {
                        session
                    } else {
                        let (cookie, session) = self.create_session_cookie().await;
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
