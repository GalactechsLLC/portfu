use async_trait::async_trait;
use http::StatusCode;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Bytes};
use log::{debug, warn};
use pfcore::service::{BodyType, IncomingRequest, MutBody};
use pfcore::wrappers::{WrapperFn, WrapperResult};
use pfcore::ServiceData;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::io::Error;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::Instant;

pub struct RateLimit {
    pub requests_count: AtomicUsize,
    pub count_seconds: AtomicUsize,
    pub request_size_limit_bytes: AtomicUsize,
}

pub struct RecentRequests {
    requests: RwLock<HashMap<String, VecDeque<Instant>>>,
    _last_request: RwLock<Instant>,
    depth: usize,
}
impl RecentRequests {
    pub fn new(depth: usize) -> Self {
        Self {
            depth,
            ..Default::default()
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

pub type ClientMap = RwLock<HashMap<String, Arc<RecentRequests>>>;
#[non_exhaustive]
pub struct RateLimiter {
    pub path_limits: Arc<RwLock<HashMap<String, Arc<RateLimit>>>>,
    pub global_limits: Arc<RateLimit>,
    pub client_rates: Arc<ClientMap>,
    pub enabled: Arc<AtomicBool>,
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
        }
    }
}
#[async_trait]
impl WrapperFn for RateLimiter {
    fn name(&self) -> &str {
        "RateLimiter"
    }
    async fn before(&self, data: &mut ServiceData) -> WrapperResult {
        let address = data
            .request
            .get()
            .expect("Expected Connection To Have SockerAddr");
        let remote = data.get_best_guess_public_ip(address);
        let recent_requests = match self.client_rates.write().await.entry(remote.clone()) {
            Entry::Vacant(e) => {
                let val = Arc::new(RecentRequests::new(
                    self.global_limits.requests_count.load(Ordering::Relaxed)
                        * self.global_limits.count_seconds.load(Ordering::Relaxed),
                ));
                e.insert(val.clone());
                val
            }
            Entry::Occupied(e) => e.get().clone(),
        };
        let (requests_per_second_limit, size_limit, recent_count) = if let Some(path_limits) = self
            .path_limits
            .read()
            .await
            .get(data.request.request.uri().path())
            .cloned()
        {
            (
                path_limits.requests_count.load(Ordering::Relaxed)
                    * path_limits.count_seconds.load(Ordering::Relaxed),
                path_limits.request_size_limit_bytes.load(Ordering::Relaxed) as u64,
                recent_requests
                    .recent_requests(
                        Some(data.request.request.uri().path()),
                        path_limits.count_seconds.load(Ordering::Relaxed) as u64,
                    )
                    .await,
            )
        } else {
            (
                self.global_limits.requests_count.load(Ordering::Relaxed)
                    * self.global_limits.count_seconds.load(Ordering::Relaxed),
                self.global_limits
                    .request_size_limit_bytes
                    .load(Ordering::Relaxed) as u64,
                recent_requests
                    .recent_requests(
                        None,
                        self.global_limits.count_seconds.load(Ordering::Relaxed) as u64,
                    )
                    .await,
            )
        };
        if recent_count >= requests_per_second_limit {
            warn!(
                "Rate limiting: {remote}, {}",
                data.request.request.uri().path()
            );
            create_error(
                data,
                format!("Too Many Requests {recent_count}, Limit is {requests_per_second_limit} / second"),
                StatusCode::TOO_MANY_REQUESTS
            )
        } else {
            debug!("Not Request Rate Limited: {recent_count} < {requests_per_second_limit}");
            recent_requests
                .add(data.request.request.uri().path().to_string())
                .await; //We only add requests we accept, so some still go through instead of overuse causing the client to always get blocked
            if size_limit > 0 {
                debug!("Checking Size Limit: {size_limit}");
                match &data.request.request {
                    IncomingRequest::Stream(stream) => {
                        let size_hint = stream.body().size_hint();
                        if let Some(size) = size_hint.exact() {
                            if size > size_limit {
                                create_error(
                                    data,
                                    format!("Payload Too large {size}, Limit is {size_limit}"),
                                    StatusCode::PAYLOAD_TOO_LARGE,
                                )
                            } else {
                                WrapperResult::Continue
                            }
                        } else if size_hint.lower() > size_limit {
                            create_error(
                                data,
                                format!(
                                    "Payload Too large {}, Limit is {size_limit}",
                                    size_hint.lower()
                                ),
                                StatusCode::PAYLOAD_TOO_LARGE,
                            )
                        } else if let Some(size) = size_hint.upper() {
                            if size > size_limit {
                                create_error(
                                    data,
                                    format!("Payload Too large {size}, Limit is {size_limit}"),
                                    StatusCode::PAYLOAD_TOO_LARGE,
                                )
                            } else {
                                WrapperResult::Continue
                            }
                        } else {
                            match handle_unsized(data, size_limit as usize).await {
                                Ok(r) => r,
                                Err(e) => create_error(
                                    data,
                                    format!("Failed to process unsized payload: {e:?}"),
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                ),
                            }
                        }
                    }
                    IncomingRequest::Sized(sized) => {
                        let size_hint = sized.body().size_hint();
                        if let Some(size) = size_hint.exact() {
                            if size > size_limit {
                                create_error(
                                    data,
                                    format!("Payload Too large {size}, Limit is {size_limit}"),
                                    StatusCode::PAYLOAD_TOO_LARGE,
                                )
                            } else {
                                WrapperResult::Continue
                            }
                        } else if size_hint.lower() > size_limit {
                            create_error(
                                data,
                                format!(
                                    "Payload Too large {}, Limit is {size_limit}",
                                    size_hint.lower()
                                ),
                                StatusCode::PAYLOAD_TOO_LARGE,
                            )
                        } else if let Some(size) = size_hint.upper() {
                            if size > size_limit {
                                create_error(
                                    data,
                                    format!("Payload Too large {size}, Limit is {size_limit}"),
                                    StatusCode::PAYLOAD_TOO_LARGE,
                                )
                            } else {
                                WrapperResult::Continue
                            }
                        } else {
                            create_error(
                                data,
                                "Failed to detect size on Sized request. Should never happen"
                                    .to_string(),
                                StatusCode::INTERNAL_SERVER_ERROR,
                            )
                        }
                    }
                    IncomingRequest::Consumed(_) => WrapperResult::Continue,
                    IncomingRequest::Empty => WrapperResult::Continue,
                }
            } else {
                WrapperResult::Continue
            }
        }
    }
    async fn after(&self, _: &mut ServiceData) -> WrapperResult {
        WrapperResult::Continue
    }
}

pub fn create_error(data: &mut ServiceData, error: String, status: StatusCode) -> WrapperResult {
    data.response
        .set_body(BodyType::Sized(Full::new(Bytes::from(error))));
    *data.response.status_mut() = status;
    WrapperResult::Return
}

#[inline]
pub async fn handle_unsized(data: &mut ServiceData, limit: usize) -> Result<WrapperResult, Error> {
    let mut body = data.request.consume();
    let mut buffer = Vec::with_capacity(limit);
    while let Some(next) = body.frame().await {
        let frame = next.map_err(|e| Error::other(format!("HTTP ERROR IN RATE_LIMITER: {e:?}")))?;
        if let Some(chunk) = frame.data_ref() {
            if buffer.len() > limit || buffer.len() + chunk.len() > limit {
                return Ok(create_error(
                    data,
                    format!("Stream Payload Too large, Limit is {limit}"),
                    StatusCode::PAYLOAD_TOO_LARGE,
                ));
            } else {
                buffer.extend(chunk);
            }
        }
    }
    data.request
        .set_body(BodyType::Sized(Full::new(Bytes::from(buffer))));
    Ok(WrapperResult::Continue)
}

impl Default for RecentRequests {
    fn default() -> Self {
        Self {
            requests: Default::default(),
            _last_request: RwLock::new(Instant::now()),
            depth: 100,
        }
    }
}
