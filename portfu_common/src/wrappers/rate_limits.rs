use crate::error::PortfuError;
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::server::Server;
use crate::server::builder::ServerBuilder;
use crate::service::request::Request;
use crate::service::response::IntoResponse;
use crate::service::response::Response;
use http::StatusCode;
use log::{debug, warn};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
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
}

impl RecentRequests {
    pub fn new(depth: usize) -> Self {
        Self {
            depth,
            requests: Default::default(),
        }
    }

    pub async fn add(&self, path: String) {
        let mut write_lock = self.requests.write().await;
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
        }
    }

    pub fn with_global_limit(global_limits: RateLimit) -> Self {
        Self::new(
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(global_limits),
        )
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
            let limit = self
                .path_limits
                .read()
                .await
                .get(path.as_str())
                .cloned()
                .unwrap_or_else(|| self.global_limits.clone());

            let limit_requests = limit.requests_count.load(Ordering::Relaxed);
            let limit_seconds = limit.count_seconds.load(Ordering::Relaxed);
            let request_window = limit_requests.saturating_mul(limit_seconds);
            let recent_requests = self.recent_requests(remote.clone(), request_window).await;
            let recent_count = recent_requests
                .recent_requests(Some(path.as_str()), limit_seconds as u64)
                .await;

            if request_window > 0 && recent_count >= request_window {
                warn!("Rate limiting TooManyRequests: {remote}, {path}");
                return Ok(MiddlewareResult::Return(Response::from_status_and_message(
                    StatusCode::TOO_MANY_REQUESTS,
                    format!("Too Many Requests {recent_count}, Limit is {request_window}"),
                )));
            }

            recent_requests.add(path.clone()).await;
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
    async fn recent_requests(&self, remote: String, depth: usize) -> Arc<RecentRequests> {
        match self.client_rates.write().await.entry(remote) {
            Entry::Vacant(e) => {
                let value = Arc::new(RecentRequests::new(depth.max(1)));
                e.insert(value.clone());
                value
            }
            Entry::Occupied(e) => e.get().clone(),
        }
    }
}

async fn enforce_body_limit(
    request: &mut Request,
    size_limit: usize,
    read_timeout: Duration,
) -> Option<Response> {
    let size_hint = request.body_size_hint();
    if size_hint
        .exact()
        .is_some_and(|size| size > size_limit as u64)
        || size_hint.lower() > size_limit as u64
        || size_hint
            .upper()
            .is_some_and(|size| size > size_limit as u64)
    {
        return Some(Response::from_status_and_message(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("Payload Too Large, Limit is {size_limit}"),
        ));
    }

    if size_hint.upper().is_some() {
        return None;
    }

    match request
        .consume_body_bytes_limited(size_limit, read_timeout)
        .await
    {
        Ok(_) => None,
        Err(e) => Some(e.into_response()),
    }
}

fn best_guess_public_ip(request: &Request) -> String {
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
#[path = "../../tests/unit/wrappers_rate_limits.rs"]
mod tests;

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
