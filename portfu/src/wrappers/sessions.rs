use async_trait::async_trait;
use cookie::Cookie;
use dashmap::DashMap;
use http::{header, Extensions, HeaderName, HeaderValue};
use once_cell::sync::Lazy;
use portfu_core::wrappers::{WrapperFn, WrapperResult};
use portfu_core::ServiceData;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

pub static SESSION_HEADER: &str = "session_id";
pub static SESSIONS: Lazy<Arc<DashMap<String, Arc<RwLock<Session>>>>> = Lazy::new(Default::default);
pub struct Session {
    pub data: Extensions,
    pub last_update: Instant,
}

pub struct SessionWrapper {
    pub session_duration: Duration,
}
impl Default for SessionWrapper {
    fn default() -> Self {
        Self {
            session_duration: Duration::from_secs(60 * 30), //30 minutes
        }
    }
}

impl SessionWrapper {
    async fn create_session_cookie(&self, data: &ServiceData) -> (Cookie, Arc<RwLock<Session>>) {
        let address: &SocketAddr = data.request.get().unwrap();
        let salt = data.get_best_guess_public_ip(address);
        let client_session_id = Uuid::new_v4();
        let mut hasher = Sha256::new();
        hasher.update([client_session_id.to_string().as_bytes(), salt.as_bytes()].concat());
        let server_session_id = hex::encode(hasher.finalize().as_slice());
        let cookie = Cookie::build((SESSION_HEADER, client_session_id.to_string()))
            .path("/")
            // .secure(true)
            .http_only(true)
            .same_site(cookie::SameSite::Lax)
            .build();
        let session = Arc::new(RwLock::new(Session {
            data: Extensions::new(),
            last_update: Instant::now(),
        }));
        SESSIONS.insert(server_session_id, session.clone());
        (cookie, session)
    }
    pub async fn get_session(
        &self,
        data: &ServiceData,
        session_cookie: Cookie<'_>,
    ) -> Option<Arc<RwLock<Session>>> {
        let address: &SocketAddr = data.request.get().unwrap();
        let salt = data.get_best_guess_public_ip(address);
        let mut hasher = Sha256::new();
        hasher.update([session_cookie.value_trimmed().as_bytes(), salt.as_bytes()].concat());
        let server_session_id = hex::encode(hasher.finalize().as_slice());
        if let Some(session) = SESSIONS.get(&server_session_id).map(|v| v.value().clone()) {
            if Instant::now().duration_since(session.read().await.last_update)
                >= self.session_duration
            {
                None
            } else {
                session.write().await.last_update = Instant::now();
                Some(session)
            }
        } else {
            None
        }
    }
}
pub fn get_session_cookie_from_request(data: &ServiceData) -> Option<Cookie> {
    let mut session_cookie = None;
    if let Some(headers) = data.request.request.headers() {
        'outer: for value in headers.get_all(header::COOKIE) {
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
    }
    session_cookie
}
#[async_trait]
impl WrapperFn for SessionWrapper {
    fn name(&self) -> &str {
        "SessionWrapper"
    }

    async fn before(&self, data: &mut ServiceData) -> WrapperResult {
        let session = match get_session_cookie_from_request(data) {
            None => {
                let (cookie, session) = self.create_session_cookie(data).await;
                if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
                    if let Some(headers) = data.request.request.headers_mut() {
                        headers.insert(HeaderName::from_static(SESSION_HEADER), value.clone());
                    }
                    data.response
                        .headers_mut()
                        .insert(header::SET_COOKIE, value);
                }
                session
            }
            Some(cookie) => {
                if let Some(session) = self.get_session(data, cookie).await {
                    session
                } else {
                    let (cookie, session) = self.create_session_cookie(data).await;
                    if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
                        if let Some(headers) = data.request.request.headers_mut() {
                            headers.insert(HeaderName::from_static(SESSION_HEADER), value.clone());
                        }
                        data.response
                            .headers_mut()
                            .insert(header::SET_COOKIE, value);
                    }
                    session
                }
            }
        };
        data.request.insert(session);
        WrapperResult::Continue
    }

    async fn after(&self, _: &mut ServiceData) -> WrapperResult {
        WrapperResult::Continue
    }
}
