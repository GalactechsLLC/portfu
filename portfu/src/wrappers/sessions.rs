use async_trait::async_trait;
use cookie::Cookie;
use dashmap::DashMap;
use http::{header, Extensions, HeaderName, HeaderValue};
use once_cell::sync::Lazy;
use pfcore::router::middleware::{Middleware, MiddlewareResult};
use pfcore::runtime::thread::ServerThread;
use pfcore::utils::signal::await_termination;
use portfu_core::ServiceData;
use std::io::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::interval;
use uuid::Uuid;

pub static SESSION_HEADER: &str = "session_id";
pub static SESSIONS: Lazy<Arc<DashMap<String, Arc<RwLock<Session>>>>> = Lazy::new(Default::default);
pub static SESSION_CLIENT_IDS: Lazy<Arc<DashMap<String, String>>> = Lazy::new(Default::default);
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
    fn build_session_cookie(&self, client_session_id: Uuid) -> Cookie<'static> {
        Cookie::build((SESSION_HEADER, client_session_id.to_string()))
            .path("/")
            .secure(self.secure)
            .http_only(true)
            .same_site(cookie::SameSite::Lax)
            .build()
            .into_owned()
    }

    async fn create_session_cookie(&self) -> (Cookie<'static>, Arc<RwLock<Session>>) {
        let client_session_id = Uuid::new_v4();
        let server_session_id = client_session_id.to_string();
        let cookie = self.build_session_cookie(client_session_id);
        let session = Arc::new(RwLock::new(Session {
            data: Extensions::new(),
            last_update: Instant::now(),
            id: client_session_id,
        }));
        SESSIONS.insert(server_session_id.clone(), session.clone());
        SESSION_CLIENT_IDS.insert(client_session_id.to_string(), server_session_id);
        (cookie, session)
    }

    pub async fn rotate_session_cookie(
        session: Arc<RwLock<Session>>,
        secure: bool,
    ) -> Cookie<'static> {
        let old_client_session_id = session.read().await.id.to_string();
        let old_server_session_id = SESSION_CLIENT_IDS
            .remove(&old_client_session_id)
            .map(|(_, value)| value)
            .unwrap_or_else(|| old_client_session_id.clone());
        SESSIONS.remove(&old_server_session_id);

        let new_client_session_id = Uuid::new_v4();
        let new_server_session_id = new_client_session_id.to_string();
        {
            let mut session_guard = session.write().await;
            session_guard.id = new_client_session_id;
            session_guard.last_update = Instant::now();
        }
        SESSIONS.insert(new_server_session_id.clone(), session);
        SESSION_CLIENT_IDS.insert(new_client_session_id.to_string(), new_server_session_id);

        Cookie::build((SESSION_HEADER, new_client_session_id.to_string()))
            .path("/")
            .secure(secure)
            .http_only(true)
            .same_site(cookie::SameSite::Lax)
            .build()
            .into_owned()
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
}
pub fn get_session_cookie_from_request(data: &ServiceData) -> Option<Cookie<'_>> {
    let mut session_cookie = None;
    'outer: for value in data.request.headers().get_all(header::COOKIE) {
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
pub async fn get_session_from_request(data: &ServiceData) -> Option<Arc<RwLock<Session>>> {
    let cookie = get_session_cookie_from_request(data)?;
    SessionManager::get_session_from_id(cookie.value_trimmed())
}
#[async_trait]
impl ServerThread for SessionManager {
    fn name(&self) -> &str {
        Middleware::name(self)
    }

    async fn run(&self, _state: Arc<RwLock<Extensions>>) -> Result<(), Error> {
        let mut interval_duration = interval(Duration::from_secs(15));
        loop {
            tokio::select! {
                _ = interval_duration.tick() => {
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
                _ = await_termination() => {
                    break;
                }
            }
        }
        Ok(())
    }
}
#[async_trait]
impl Middleware for SessionManager {
    fn name(&self) -> &str {
        "SessionManager"
    }

    async fn before(&self, data: &mut ServiceData) -> Result<MiddlewareResult, Error> {
        let session = match get_session_cookie_from_request(data) {
            None => {
                let (cookie, session) = self.create_session_cookie().await;
                if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
                    data.request
                        .headers_mut()
                        .insert(HeaderName::from_static(SESSION_HEADER), value.clone());
                    data.response
                        .headers_mut()
                        .insert(header::SET_COOKIE, value);
                }
                session
            }
            Some(cookie) => {
                if let Some(session) = self.get_session(cookie).await {
                    session
                } else {
                    let (cookie, session) = self.create_session_cookie().await;
                    if let Ok(value) = HeaderValue::from_str(&cookie.to_string()) {
                        data.request
                            .headers_mut()
                            .insert(HeaderName::from_static(SESSION_HEADER), value.clone());
                        data.response
                            .headers_mut()
                            .insert(header::SET_COOKIE, value);
                    }
                    session
                }
            }
        };
        data.request.insert(session);
        Ok(MiddlewareResult::Continue)
    }

    async fn after(&self, _: &mut ServiceData) -> Result<MiddlewareResult, Error> {
        Ok(MiddlewareResult::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rotate_session_cookie_replaces_client_session_id_and_preserves_data() {
        let manager = SessionManager {
            session_duration: Duration::from_secs(60),
            secure: true,
        };
        let (_cookie, session) = manager.create_session_cookie().await;
        let old_id = session.read().await.id.to_string();
        session
            .write()
            .await
            .data
            .insert("authenticated".to_string());

        let replacement = SessionManager::rotate_session_cookie(session.clone(), true).await;
        let new_id = session.read().await.id.to_string();

        assert_ne!(old_id, new_id);
        assert_eq!(replacement.value_trimmed(), new_id);
        assert!(SessionManager::get_session_from_id(&old_id).is_none());
        let rotated =
            SessionManager::get_session_from_id(&new_id).expect("new session id is mapped");
        assert_eq!(
            rotated
                .read()
                .await
                .data
                .get::<String>()
                .map(String::as_str),
            Some("authenticated")
        );
    }
}
