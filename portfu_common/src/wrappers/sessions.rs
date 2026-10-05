use crate::error::PortfuError;
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::server::builder::ServerBuilder;
use crate::service::request::{FromRequest, Request};
use crate::service::response::Response;
use cookie::Cookie;
use dashmap::DashMap;
use http::header;
use http::{Extensions, HeaderValue};
use once_cell::sync::Lazy;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

pub static SESSION_HEADER: &str = "session_id";
pub static SESSIONS: Lazy<Arc<DashMap<String, Arc<RwLock<Session>>>>> = Lazy::new(Default::default);
pub static SESSION_CLIENT_IDS: Lazy<Arc<DashMap<String, String>>> = Lazy::new(Default::default);

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

/// A backend owns expiration and persistence. Shared backends serialize the application's
/// known extension types and must atomically reject saves of revoked session IDs.
pub trait SessionStore: Send + Sync {
    fn load<'a>(&'a self, id: Uuid) -> StoreFuture<'a, Option<Arc<RwLock<Session>>>>;
    fn save<'a>(&'a self, session: Arc<RwLock<Session>>, ttl: Duration) -> StoreFuture<'a, ()>;
    fn remove<'a>(&'a self, id: Uuid) -> StoreFuture<'a, ()>;
    fn cleanup<'a>(&'a self) -> StoreFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn maintain<'a>(&'a self) -> StoreFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

pub type StoreFuture<'a, T> =
    std::pin::Pin<Box<dyn Future<Output = Result<T, PortfuError>> + Send + Sync + 'a>>;

/// Process-local storage with a hard capacity. Use a shared SessionStore for replicas.
pub struct MemorySessionStore {
    sessions: Arc<DashMap<String, Arc<RwLock<Session>>>>,
    client_ids: Arc<DashMap<String, String>>,
    ttls: Arc<DashMap<String, Duration>>,
    capacity: usize,
    admission: tokio::sync::Mutex<()>,
    last_cleanup: std::sync::Mutex<Instant>,
}

impl MemorySessionStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            client_ids: Arc::new(DashMap::new()),
            ttls: Arc::new(DashMap::new()),
            capacity,
            admission: tokio::sync::Mutex::new(()),
            last_cleanup: std::sync::Mutex::new(Instant::now()),
        }
    }

    async fn sweep(&self) {
        // Never hold a DashMap guard across an await.
        let sessions: Vec<_> = self
            .sessions
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        for (key, session) in sessions {
            let ttl = self.ttls.get(&key).map(|v| *v).unwrap_or_default();
            let mut state = session.write().await;
            if state.last_update.elapsed() >= ttl {
                state.id = Uuid::nil();
                self.sessions.remove(&key);
                self.client_ids.remove(&key);
                self.ttls.remove(&key);
            }
        }
    }
}

impl SessionStore for MemorySessionStore {
    fn load<'a>(&'a self, id: Uuid) -> StoreFuture<'a, Option<Arc<RwLock<Session>>>> {
        Box::pin(async move {
            let key = id.to_string();
            let session = self.sessions.get(&key).map(|v| v.value().clone());
            if let Some(session) = session {
                let ttl = self.ttls.get(&key).map(|v| *v).unwrap_or_default();
                let mut state = session.write().await;
                if state.last_update.elapsed() >= ttl || state.id != id {
                    state.id = Uuid::nil();
                    drop(state);
                    self.remove(id).await?;
                    return Ok(None);
                }
                state.last_update = Instant::now();
                drop(state);
                Ok(Some(session))
            } else {
                Ok(None)
            }
        })
    }

    fn save<'a>(&'a self, session: Arc<RwLock<Session>>, ttl: Duration) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let _admission = self.admission.lock().await;
            let mut state = session.write().await;
            let key = state.id.to_string();
            // Rotation invalidates the old object as well as its index.
            if state.id.is_nil() {
                return Ok(());
            }
            if !self.sessions.contains_key(&key) && self.sessions.len() >= self.capacity {
                return Err(PortfuError::ServiceUnavailable(
                    "Session capacity reached".into(),
                ));
            }
            if state.id.is_nil() || state.id.to_string() != key {
                return Ok(());
            }
            state.last_update = Instant::now();
            self.ttls.insert(key.clone(), ttl);
            self.sessions.insert(key.clone(), session.clone());
            self.client_ids.insert(key.clone(), key);
            Ok(())
        })
    }

    fn remove<'a>(&'a self, id: Uuid) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let _admission = self.admission.lock().await;
            let key = id.to_string();
            let removed = self.sessions.remove(&key).map(|(_, session)| session);
            if let Some(session) = removed {
                session.write().await.id = Uuid::nil();
            }
            self.client_ids.remove(&key);
            self.ttls.remove(&key);
            Ok(())
        })
    }

    fn cleanup<'a>(&'a self) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let _admission = self.admission.lock().await;
            self.sweep().await;
            Ok(())
        })
    }
    fn maintain<'a>(&'a self) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let should_clean = {
                let mut last = self.last_cleanup.lock().unwrap();
                if last.elapsed() >= Duration::from_secs(60) {
                    *last = Instant::now();
                    true
                } else {
                    false
                }
            };
            if should_clean {
                self.cleanup().await?;
            }
            Ok(())
        })
    }
}

static DEFAULT_STORE: Lazy<Arc<MemorySessionStore>> = Lazy::new(|| {
    let mut store = MemorySessionStore::new(10_000);
    store.sessions = SESSIONS.clone();
    store.client_ids = SESSION_CLIENT_IDS.clone();
    Arc::new(store)
});

type SessionBypassPredicate = Arc<dyn Fn(&Request) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct SessionManager {
    pub session_duration: Duration,
    pub secure: bool,
    store: Arc<dyn SessionStore>,
    skip_when: Option<SessionBypassPredicate>,
    middleware_id: Uuid,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(Duration::from_secs(60 * 30), true)
    }
}

#[derive(Clone)]
struct SessionContext {
    manager: SessionManager,
    original_id: Option<Uuid>,
}

/// Insert this marker before SessionManager, or call Request::skip_session().
#[derive(Clone, Copy)]
pub struct SkipSession;

impl Request {
    /// Suppress session loading, persistence and cookies for this request.
    pub fn skip_session(&mut self) {
        self.insert(SkipSession);
        self.remove::<Arc<RwLock<Session>>>();
    }
}

impl SessionManager {
    pub fn new(session_duration: Duration, secure: bool) -> Self {
        Self {
            session_duration,
            secure,
            store: DEFAULT_STORE.clone(),
            skip_when: None,
            middleware_id: Uuid::new_v4(),
        }
    }

    /// Bypass sessions for application-defined authentication, such as API-key requests.
    pub fn skip_when(
        mut self,
        predicate: impl Fn(&Request) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.skip_when = Some(Arc::new(predicate));
        self
    }

    pub fn store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.store = store;
        self
    }

    pub async fn get_session(
        &self,
        _request: &Request,
        cookie: Cookie<'_>,
    ) -> Option<Arc<RwLock<Session>>> {
        self.store
            .load(Uuid::parse_str(cookie.value_trimmed()).ok()?)
            .await
            .ok()
            .flatten()
    }

    /// Lookup in the default process-local backend only.
    pub fn get_session_from_id(client_id: &str) -> Option<Arc<RwLock<Session>>> {
        let key = SESSION_CLIENT_IDS
            .get(client_id)
            .map(|v| v.value().clone())?;
        let session = SESSIONS.get(&key).map(|v| v.value().clone());
        if let Some(session) = &session {
            let state = session.try_read().ok()?;
            let ttl = DEFAULT_STORE.ttls.get(&key).map(|v| *v).unwrap_or_default();
            if state.id.is_nil() || state.last_update.elapsed() >= ttl {
                return None;
            }
        } else {
            SESSION_CLIENT_IDS.remove(client_id);
        }
        session
    }

    pub async fn cleanup_expired(&self) {
        let _ = self.store.cleanup().await;
    }

    /// Revoke the previous ID and detach its object before adding authenticated data.
    pub async fn rotate_session(request: &mut Request) -> Result<(), PortfuError> {
        let context = request
            .get::<SessionContext>()
            .cloned()
            .ok_or_else(|| PortfuError::Unauthorized("No managed session".into()))?;
        let old = request
            .get::<Arc<RwLock<Session>>>()
            .cloned()
            .ok_or_else(|| PortfuError::Unauthorized("No active session".into()))?;
        let mut state = old.write().await;
        let id = state.id;
        let data = std::mem::take(&mut state.data);
        state.id = Uuid::nil();
        drop(state);
        context.manager.store.remove(id).await?;
        request.insert(Arc::new(RwLock::new(Session {
            data,
            ..Session::default()
        })));
        Ok(())
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

    pub fn store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.manager.store = store;
        self
    }

    pub fn skip_when(
        mut self,
        predicate: impl Fn(&Request) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.manager = self.manager.skip_when(predicate);
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
    if request.get::<SkipSession>().is_some() {
        return None;
    }
    if let Some(session) = request.get::<Arc<RwLock<Session>>>() {
        return Some(session.clone());
    }
    let cookie = get_session_cookie_from_request(request)?;
    if let Some(server) = request.get::<Arc<crate::server::Server>>() {
        let manager = server
            .middleware
            .iter()
            .find_map(|middleware| middleware.session_manager())?;
        if manager
            .skip_when
            .as_ref()
            .is_some_and(|predicate| predicate(request))
        {
            return None;
        }
        return manager.get_session(request, cookie).await;
    }
    DEFAULT_STORE
        .load(Uuid::parse_str(cookie.value_trimmed()).ok()?)
        .await
        .ok()
        .flatten()
}

impl Middleware for SessionManager {
    fn name(&self) -> &str {
        "SessionManager"
    }

    fn is_session_manager(&self) -> bool {
        true
    }

    fn session_manager(&self) -> Option<&SessionManager> {
        Some(self)
    }

    fn before<'a>(&'a self, request: &'a mut Request) -> StoreFuture<'a, MiddlewareResult> {
        Box::pin(async move {
            if self
                .skip_when
                .as_ref()
                .is_some_and(|predicate| predicate(request))
            {
                request.skip_session();
            }
            if request.get::<SkipSession>().is_some() || request.get::<SessionContext>().is_some() {
                return Ok(MiddlewareResult::Continue);
            }
            self.store.maintain().await?;
            let original = match get_session_cookie_from_request(request)
                .and_then(|cookie| Uuid::parse_str(cookie.value_trimmed()).ok())
            {
                Some(id) => self.store.load(id).await?,
                None => None,
            };
            let original_id = match &original {
                Some(session) => Some(session.read().await.id),
                None => None,
            };
            request.insert(original.unwrap_or_else(|| Arc::new(RwLock::new(Session::default()))));
            request.insert(SessionContext {
                manager: self.clone(),
                original_id,
            });
            Ok(MiddlewareResult::Continue)
        })
    }

    fn after_with_request<'a>(
        &'a self,
        request: &'a Request,
        response: &'a mut Response,
    ) -> StoreFuture<'a, MiddlewareResult> {
        Box::pin(async move {
            if request.get::<SkipSession>().is_some() {
                return Ok(MiddlewareResult::Continue);
            }
            let Some(context) = request.get::<SessionContext>() else {
                return Ok(MiddlewareResult::Continue);
            };
            // Only the manager that attached the context finalizes the session.
            if self.middleware_id != context.manager.middleware_id {
                return Ok(MiddlewareResult::Continue);
            }
            let Some(session) = request.get::<Arc<RwLock<Session>>>().cloned() else {
                return Ok(MiddlewareResult::Continue);
            };
            let state = session.read().await;
            if state.id.is_nil() || (context.original_id.is_none() && state.data.is_empty()) {
                return Ok(MiddlewareResult::Continue);
            }
            let id = state.id;
            drop(state);
            self.store.save(session, self.session_duration).await?;
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            );
            if context.original_id != Some(id) {
                let cookie = Cookie::build((SESSION_HEADER, id.to_string()))
                    .path("/")
                    .secure(self.secure)
                    .http_only(true)
                    .same_site(cookie::SameSite::Lax)
                    .build();
                let value = HeaderValue::from_str(&cookie.to_string())
                    .map_err(|_| PortfuError::Internal("Invalid session cookie".into()))?;
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            Ok(MiddlewareResult::Continue)
        })
    }

    fn after<'a>(&'a self, _: &'a mut Response) -> StoreFuture<'a, MiddlewareResult> {
        Box::pin(async { Ok(MiddlewareResult::Continue) })
    }
}
