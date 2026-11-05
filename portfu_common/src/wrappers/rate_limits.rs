use crate::error::PortfuError;
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::server::builder::ServerBuilder;
use crate::service::request::Request;
use crate::service::response::IntoResponse;
use crate::service::response::Response;
use http::StatusCode;
use log::{debug, warn};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::Instant;

pub struct RateLimit {
    pub requests_count: AtomicUsize,
    pub count_seconds: AtomicUsize,
    pub request_size_limit_bytes: AtomicUsize,
}

impl RateLimit {
    pub fn new(
        requests_count: usize,
        count_seconds: usize,
        request_size_limit_bytes: usize,
    ) -> Self {
        Self {
            requests_count: AtomicUsize::new(requests_count),
            count_seconds: AtomicUsize::new(count_seconds),
            request_size_limit_bytes: AtomicUsize::new(request_size_limit_bytes),
        }
    }
}

impl Default for RateLimit {
    fn default() -> Self {
        Self::new(120, 60, 1024 * 1024)
    }
}

pub struct RecentRequests {
    requests: RwLock<HashMap<String, VecDeque<Instant>>>,
    depth: usize,
    expires_at: Mutex<Instant>,
    windows: RwLock<HashMap<String, (Duration, VecDeque<Instant>)>>,
}

impl RecentRequests {
    pub fn new(depth: usize) -> Self {
        Self {
            depth,
            requests: Default::default(),
            expires_at: Mutex::new(Instant::now()),
            windows: RwLock::new(HashMap::new()),
        }
    }

    pub async fn add(&self, path: String) {
        let mut write_lock = self.requests.write().await;
        if !write_lock.contains_key(&path) && write_lock.len() >= 64 {
            return;
        }
        match write_lock.entry(path) {
            Entry::Occupied(mut e) => {
                e.get_mut().push_front(Instant::now());
                e.get_mut().truncate(self.depth);
            }
            Entry::Vacant(e) => {
                e.insert(VecDeque::from([Instant::now()]));
            }
        }
    }

    async fn admit(&self, limits: &[(String, usize, Duration)], max_buckets: usize) -> bool {
        let now = Instant::now();
        let mut windows = self.windows.write().await;
        windows.retain(|_, (window, times)| {
            while times
                .back()
                .is_some_and(|time| now.duration_since(*time) >= *window)
            {
                times.pop_back();
            }
            !times.is_empty()
        });
        let missing = limits
            .iter()
            .filter(|(key, count, _)| *count > 0 && !windows.contains_key(key))
            .count();
        if windows.len().saturating_add(missing) > max_buckets {
            return false;
        }
        for (key, count, window) in limits {
            if *count == 0 {
                continue;
            }
            if windows.get(key).is_some_and(|(_, times)| {
                times
                    .iter()
                    .filter(|time| now.duration_since(**time) < *window)
                    .count()
                    >= *count
            }) {
                return false;
            }
        }
        for (key, count, window) in limits {
            if *count == 0 {
                continue;
            }
            let entry = windows
                .entry(key.clone())
                .or_insert_with(|| (*window, VecDeque::new()));
            entry.0 = *window;
            entry.1.push_front(now);
            entry.1.truncate(*count);
        }
        true
    }

    pub async fn recent_requests(&self, path: Option<&str>, last_seconds: u64) -> usize {
        let now = Instant::now();
        match path {
            None => self
                .requests
                .read()
                .await
                .values()
                .map(|v| {
                    v.iter()
                        .filter(|i| now.saturating_duration_since(**i).as_secs() <= last_seconds)
                        .count()
                })
                .sum::<usize>(),
            Some(path) => self
                .requests
                .read()
                .await
                .get(path)
                .map(|v| {
                    v.iter()
                        .filter(|i| now.saturating_duration_since(**i).as_secs() <= last_seconds)
                        .count()
                })
                .unwrap_or_default(),
        }
    }
}

impl Default for RecentRequests {
    fn default() -> Self {
        Self::new(100)
    }
}

pub type ClientMap = RwLock<HashMap<String, Arc<RecentRequests>>>;

#[non_exhaustive]
pub struct RateLimiter {
    pub path_limits: Arc<RwLock<HashMap<String, Arc<RateLimit>>>>,
    pub global_limits: Arc<RateLimit>,
    pub client_rates: Arc<ClientMap>,
    pub enabled: Arc<AtomicBool>,
    pub body_read_timeout: Duration,
    pub max_clients: usize,
    pub max_buckets_per_client: usize,
    pub client_idle_timeout: Duration,
    last_cleanup: Mutex<Instant>,
}

impl RateLimiter {
    pub fn new(
        client_rates: Arc<ClientMap>,
        path_limits: Arc<RwLock<HashMap<String, Arc<RateLimit>>>>,
        global_limits: Arc<RateLimit>,
    ) -> Self {
        Self {
            path_limits,
            global_limits,
            client_rates,
            enabled: Arc::new(AtomicBool::new(true)),
            body_read_timeout: Duration::from_secs(5),
            max_clients: 10_000,
            max_buckets_per_client: 64,
            client_idle_timeout: Duration::from_secs(300),
            last_cleanup: Mutex::new(Instant::now()),
        }
    }

    pub fn with_global_limit(global_limits: RateLimit) -> Self {
        Self::new(
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(global_limits),
        )
    }

    pub fn capacity(mut self, clients: usize, buckets_per_client: usize) -> Self {
        self.max_clients = clients;
        self.max_buckets_per_client = buckets_per_client;
        self
    }

    pub fn client_idle_timeout(mut self, timeout: Duration) -> Self {
        self.client_idle_timeout = timeout;
        self
    }

    pub fn body_read_timeout(mut self, timeout: Duration) -> Self {
        self.body_read_timeout = timeout;
        self
    }

    pub fn request_size_limit(self, bytes: usize) -> Self {
        self.global_limits
            .request_size_limit_bytes
            .store(bytes, Ordering::Relaxed);
        self
    }

    pub async fn cleanup_expired(&self) {
        let now = Instant::now();
        self.client_rates
            .write()
            .await
            .retain(|_, recent| *recent.expires_at.lock().unwrap() > now);
    }

    pub async fn set_path_limit<S: Into<String>>(&self, path: S, limit: RateLimit) {
        self.path_limits
            .write()
            .await
            .insert(path.into(), Arc::new(limit));
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::with_global_limit(RateLimit::default())
    }
}

impl Middleware for RateLimiter {
    fn name(&self) -> &str {
        "RateLimiter"
    }

    fn before<'a>(
        &'a self,
        request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move {
            if !self.enabled.load(Ordering::Relaxed) {
                return Ok(MiddlewareResult::Continue);
            }

            let remote = best_guess_public_ip(request);
            let path = request.uri().path().to_string();
            let configured = self.path_limits.read().await;
            // Exact rules take precedence; templates group all content IDs together.
            let rule = configured.get_key_value(path.as_str()).or_else(|| {
                configured
                    .iter()
                    .filter(|(pattern, _)| {
                        crate::router::route::Route::new((*pattern).clone()).matches(path.as_str())
                    })
                    .min_by(|(left, _), (right, _)| left.cmp(right))
            });
            let mut limits = vec![(
                "global".to_string(),
                self.global_limits.requests_count.load(Ordering::Relaxed),
                Duration::from_secs(self.global_limits.count_seconds.load(Ordering::Relaxed) as u64),
            )];
            let limit = if let Some((pattern, limit)) = rule {
                limits.push((
                    format!("path:{pattern}"),
                    limit.requests_count.load(Ordering::Relaxed),
                    Duration::from_secs(limit.count_seconds.load(Ordering::Relaxed) as u64),
                ));
                limit.clone()
            } else {
                self.global_limits.clone()
            };
            drop(configured);
            let retention = limits
                .iter()
                .map(|(_, _, window)| *window)
                .max()
                .unwrap_or_default()
                .max(self.client_idle_timeout);
            let accepted = match self.recent_requests(remote.clone(), retention).await {
                Some(recent) => recent.admit(&limits, self.max_buckets_per_client).await,
                None => false,
            };
            if !accepted {
                return Ok(MiddlewareResult::Return(Response::from_status_and_message(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too Many Requests",
                )));
            }
            let size_limit = limit.request_size_limit_bytes.load(Ordering::Relaxed);
            if size_limit == 0 {
                return Ok(MiddlewareResult::Continue);
            }

            if let Some(response) =
                enforce_body_limit(request, size_limit, self.body_read_timeout).await
            {
                warn!("Rate limiting request body: {remote}, {path}");
                Ok(MiddlewareResult::Return(response))
            } else {
                debug!("Request accepted by rate limiter: {remote}, {path}");
                Ok(MiddlewareResult::Continue)
            }
        })
    }

    fn after<'a>(
        &'a self,
        _response: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move { Ok(MiddlewareResult::Continue) })
    }
}

impl RateLimiter {
    async fn recent_requests(
        &self,
        remote: String,
        retention: Duration,
    ) -> Option<Arc<RecentRequests>> {
        let now = Instant::now();
        let mut clients = self.client_rates.write().await;
        let cleanup = {
            let mut last = self.last_cleanup.lock().unwrap();
            if now.duration_since(*last) >= Duration::from_secs(60) {
                *last = now;
                true
            } else {
                false
            }
        };
        if cleanup {
            clients.retain(|_, recent| *recent.expires_at.lock().unwrap() > now);
        }
        if !clients.contains_key(&remote) && clients.len() >= self.max_clients {
            return None;
        }
        let recent = clients
            .entry(remote)
            .or_insert_with(|| Arc::new(RecentRequests::new(1)))
            .clone();
        let mut expires_at = recent.expires_at.lock().unwrap();
        *expires_at = (*expires_at).max(now + retention);
        drop(expires_at);
        Some(recent)
    }
}

async fn enforce_body_limit(
    request: &mut Request,
    size_limit: usize,
    read_timeout: Duration,
) -> Option<Response> {
    request
        .limit_body(size_limit, read_timeout)
        .err()
        .map(IntoResponse::into_response)
}

pub(crate) fn best_guess_public_ip(request: &Request) -> String {
    crate::service::request::client_ip(request).to_string()
}

pub struct RateLimitServerBuilder {
    builder: ServerBuilder,
    limiter: RateLimiter,
}

impl RateLimitServerBuilder {
    pub fn requests_per_window(self, requests: usize, seconds: usize) -> Self {
        self.limiter
            .global_limits
            .requests_count
            .store(requests, Ordering::Relaxed);
        self.limiter
            .global_limits
            .count_seconds
            .store(seconds, Ordering::Relaxed);
        self
    }

    pub fn request_size_limit(self, bytes: usize) -> Self {
        self.limiter
            .global_limits
            .request_size_limit_bytes
            .store(bytes, Ordering::Relaxed);
        self
    }

    pub fn body_read_timeout(mut self, timeout: Duration) -> Self {
        self.limiter.body_read_timeout = timeout;
        self
    }

    pub fn capacity(mut self, clients: usize, buckets_per_client: usize) -> Self {
        self.limiter = self.limiter.capacity(clients, buckets_per_client);
        self
    }

    pub fn client_idle_timeout(mut self, timeout: Duration) -> Self {
        self.limiter.client_idle_timeout = timeout;
        self
    }

    pub fn finish_rate_limits(self) -> ServerBuilder {
        self.builder.wrap(Arc::new(self.limiter))
    }

    pub fn build(self) -> crate::server::Server {
        self.finish_rate_limits().build()
    }
}

impl ServerBuilder {
    pub fn enable_rate_limits(self) -> RateLimitServerBuilder {
        RateLimitServerBuilder {
            builder: self,
            limiter: RateLimiter::default(),
        }
    }

    pub fn rate_limiter(self, limiter: RateLimiter) -> Self {
        self.wrap(Arc::new(limiter))
    }
}
