use crate::error::PortfuError;
use crate::router::route::Route;
use crate::service::response::IntoResponse;
use crate::service::{DEFAULT_URI, PinnedBody, StreamingBody};
use http::request::Parts;
use http::{Extensions, HeaderMap, HeaderValue, Method, Uri};
use http_body::Body as HttpBody;
use http_body_util::Full;
use http_body_util::{BodyExt, BodyStream, StreamBody};
use hyper::body::{Bytes, Frame, SizeHint};
use serde::de::DeserializeOwned;
use std::mem::replace;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::{Sleep, sleep};

pub enum RequestType {
    Stream(http::Request<StreamingBody>),
    Sized(http::Request<Full<Bytes>>),
    Consumed(Parts),
    Empty(HeaderMap<HeaderValue>),
}
pub trait FromRequest<T>: Sized {
    type Error: IntoResponse;
    fn try_from<'a>(
        value: &'a mut T,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>>;
}
pub struct Request {
    request_type: RequestType,
    route: Arc<Route>,
}
impl Request {
    pub fn new(request_type: RequestType, route: Arc<Route>) -> Self {
        Request {
            request_type,
            route,
        }
    }
    pub fn request_type(&mut self) -> &mut RequestType {
        &mut self.request_type
    }
    pub fn route(&self) -> Arc<Route> {
        self.route.clone()
    }
    pub fn route_mut(&mut self) -> &mut Arc<Route> {
        &mut self.route
    }

    pub fn uri(&self) -> &Uri {
        match &self.request_type {
            RequestType::Sized(r) => r.uri(),
            RequestType::Stream(r) => r.uri(),
            RequestType::Consumed(r) => &r.uri,
            RequestType::Empty(_) => &DEFAULT_URI,
        }
    }
    pub fn shared_state_mut(&mut self) -> Option<&mut Extensions> {
        match &mut self.request_type {
            RequestType::Sized(r) => Some(r.extensions_mut()),
            RequestType::Stream(r) => Some(r.extensions_mut()),
            RequestType::Consumed(r) => Some(&mut r.extensions),
            RequestType::Empty(_) => None,
        }
    }
    pub fn method(&self) -> &Method {
        match &self.request_type {
            RequestType::Sized(r) => r.method(),
            RequestType::Stream(r) => r.method(),
            RequestType::Consumed(r) => &r.method,
            RequestType::Empty(_) => &Method::OPTIONS,
        }
    }
    pub fn host(&self) -> Option<&str> {
        let from_headers = self
            .headers()
            .get(http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(':').next().unwrap_or(v));
        from_headers.or_else(|| self.uri().host())
    }
    pub fn headers(&self) -> &HeaderMap<HeaderValue> {
        match &self.request_type {
            RequestType::Sized(r) => r.headers(),
            RequestType::Stream(r) => r.headers(),
            RequestType::Consumed(r) => &r.headers,
            RequestType::Empty(h) => h,
        }
    }
    pub fn headers_mut(&mut self) -> &mut HeaderMap<HeaderValue> {
        match &mut self.request_type {
            RequestType::Sized(r) => r.headers_mut(),
            RequestType::Stream(r) => r.headers_mut(),
            RequestType::Consumed(r) => &mut r.headers,
            RequestType::Empty(h) => h,
        }
    }
    pub fn body_size_hint(&self) -> SizeHint {
        match &self.request_type {
            RequestType::Stream(r) => r.body().size_hint(),
            RequestType::Sized(r) => r.body().size_hint(),
            RequestType::Consumed(_) | RequestType::Empty(_) => SizeHint::with_exact(0),
        }
    }
    pub fn set_body_bytes(&mut self, bytes: Bytes) {
        match replace(&mut self.request_type, RequestType::Empty(HeaderMap::new())) {
            RequestType::Stream(r) => {
                let (parts, _) = r.into_parts();
                self.request_type =
                    RequestType::Sized(http::Request::from_parts(parts, Full::new(bytes)));
            }
            RequestType::Sized(r) => {
                let (parts, _) = r.into_parts();
                self.request_type =
                    RequestType::Sized(http::Request::from_parts(parts, Full::new(bytes)));
            }
            RequestType::Consumed(parts) => {
                self.request_type =
                    RequestType::Sized(http::Request::from_parts(parts, Full::new(bytes)));
            }
            RequestType::Empty(mut headers) => {
                if bytes.is_empty() {
                    self.request_type = RequestType::Empty(headers);
                } else {
                    let mut request = http::Request::new(Full::new(bytes));
                    request.headers_mut().extend(headers.drain());
                    self.request_type = RequestType::Sized(request);
                }
            }
        }
    }
    fn body_limits(&self) -> BodyLimits {
        let server = self.get::<Arc<crate::server::Server>>();
        let mut limits = BodyLimits {
            max_bytes: server.map_or(1024 * 1024, |s| s.config.request_size_limit_bytes),
            timeout: server.map_or(Duration::from_secs(30), |s| s.config.body_read_timeout),
        };
        if let Some(local) = self.get::<BodyLimits>() {
            limits.max_bytes = limits.max_bytes.min(local.max_bytes);
            limits.timeout = limits.timeout.min(local.timeout);
        }
        limits
    }

    /// Reject a known oversized body without polling or buffering it.
    pub fn check_body_limit(&self) -> Result<(), PortfuError> {
        let limit = self.body_limits().max_bytes as u64;
        if self.body_size_hint().lower() > limit {
            return Err(PortfuError::PayloadTooLarge(
                "Request body exceeded limit".into(),
            ));
        }
        if let Some(value) = self.headers().get(http::header::CONTENT_LENGTH) {
            let length = value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or_else(|| PortfuError::BadRequest("Invalid Content-Length".into()))?;
            if length > limit {
                return Err(PortfuError::PayloadTooLarge(
                    "Request body exceeded limit".into(),
                ));
            }
        }
        Ok(())
    }

    /// Tighten body limits without reading any bytes. Server limits remain an upper bound.
    pub fn limit_body(
        &mut self,
        max_bytes: usize,
        read_timeout: Duration,
    ) -> Result<(), PortfuError> {
        let current = self.body_limits();
        self.insert(BodyLimits {
            max_bytes: current.max_bytes.min(max_bytes),
            timeout: current.timeout.min(read_timeout),
        });
        self.check_body_limit()?;
        // Wrap the raw stream as well, so custom handlers using request_type() cannot
        // accidentally bypass the configured cap. No body frame is polled here.
        if matches!(self.request_type, RequestType::Stream(_)) {
            let limits = self.body_limits();
            let RequestType::Stream(raw) =
                replace(&mut self.request_type, RequestType::Empty(HeaderMap::new()))
            else {
                unreachable!()
            };
            let (parts, body) = raw.into_parts();
            let bounded = RequestBody {
                body: Box::pin(body),
                remaining: limits.max_bytes,
                read_timeout: limits.timeout,
                deadline: None,
                finished: false,
            };
            let body: PinnedBody = Box::pin(bounded.map_err(|error| match error {
                PortfuError::PayloadTooLarge(_) => "portfu:payload-too-large",
                PortfuError::RequestTimeout(_) => "portfu:request-timeout",
                _ => "portfu:bad-request",
            }));
            self.request_type = RequestType::Stream(http::Request::from_parts(
                parts,
                StreamBody::new(BodyStream::new(body)),
            ));
        }
        Ok(())
    }

    /// Take the body stream, preserving request headers and extensions for other extractors.
    pub fn take_body(&mut self) -> Result<RequestBody, PortfuError> {
        self.check_body_limit()?;
        let limits = self.body_limits();
        let body: PinnedBody =
            match replace(&mut self.request_type, RequestType::Empty(HeaderMap::new())) {
                RequestType::Stream(r) => {
                    let (parts, body) = r.into_parts();
                    self.request_type = RequestType::Consumed(parts);
                    Box::pin(body)
                }
                RequestType::Sized(r) => {
                    let (parts, body) = r.into_parts();
                    self.request_type = RequestType::Consumed(parts);
                    Box::pin(body.map_err(|never| match never {}))
                }
                RequestType::Consumed(parts) => {
                    self.request_type = RequestType::Consumed(parts);
                    Box::pin(Full::new(Bytes::new()).map_err(|never| match never {}))
                }
                RequestType::Empty(headers) => {
                    self.request_type = RequestType::Empty(headers);
                    Box::pin(Full::new(Bytes::new()).map_err(|never| match never {}))
                }
            };
        Ok(RequestBody {
            body,
            remaining: limits.max_bytes,
            read_timeout: limits.timeout,
            deadline: None,
            finished: false,
        })
    }

    pub async fn consume_body_bytes(&mut self) -> Result<Bytes, PortfuError> {
        Ok(self.take_body()?.collect().await?.to_bytes())
    }

    pub async fn consume_body_bytes_limited(
        &mut self,
        max_bytes: usize,
        read_timeout: Duration,
    ) -> Result<Bytes, PortfuError> {
        self.limit_body(max_bytes, read_timeout)?;
        let bytes = self.consume_body_bytes().await?;
        self.set_body_bytes(bytes.clone());
        Ok(bytes)
    }

    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        match &self.request_type {
            RequestType::Sized(r) => r.extensions().get::<T>(),
            RequestType::Stream(r) => r.extensions().get::<T>(),
            RequestType::Consumed(r) => r.extensions.get::<T>(),
            RequestType::Empty(_) => None,
        }
    }
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        match &mut self.request_type {
            RequestType::Sized(r) => r.extensions_mut().get_mut::<T>(),
            RequestType::Stream(r) => r.extensions_mut().get_mut::<T>(),
            RequestType::Consumed(r) => r.extensions.get_mut::<T>(),
            RequestType::Empty(_) => None,
        }
    }
    pub fn insert<T: Clone + Send + Sync + 'static>(&mut self, value: T) -> Option<T> {
        match &mut self.request_type {
            RequestType::Sized(r) => Some(r.extensions_mut().insert(value)).flatten(),
            RequestType::Stream(r) => Some(r.extensions_mut().insert(value)).flatten(),
            RequestType::Consumed(r) => Some(r.extensions.insert(value)).flatten(),
            RequestType::Empty(_) => None,
        }
    }
    pub fn remove<T: Clone + Send + Sync + 'static>(&mut self) -> Option<T> {
        match &mut self.request_type {
            RequestType::Sized(r) => r.extensions_mut().remove::<T>(),
            RequestType::Stream(r) => r.extensions_mut().remove::<T>(),
            RequestType::Consumed(r) => r.extensions.remove::<T>(),
            RequestType::Empty(_) => None,
        }
    }
}

#[derive(Clone, Copy)]
struct BodyLimits {
    max_bytes: usize,
    timeout: Duration,
}

/// An unbuffered request body. Each frame is bounded and subject to an idle timeout.
/// Use BodyExt::frame() to consume it incrementally in an upload handler.
pub struct RequestBody {
    body: PinnedBody,
    remaining: usize,
    read_timeout: Duration,
    deadline: Option<Pin<Box<Sleep>>>,
    finished: bool,
}

impl HttpBody for RequestBody {
    type Data = Bytes;
    type Error = PortfuError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, PortfuError>>> {
        if self.finished {
            return Poll::Ready(None);
        }
        if self.deadline.is_none() {
            self.deadline = Some(Box::pin(sleep(self.read_timeout)));
        }
        if self.deadline.as_mut().unwrap().as_mut().poll(cx).is_ready() {
            self.finished = true;
            return Poll::Ready(Some(Err(PortfuError::RequestTimeout(
                "Body read timeout".into(),
            ))));
        }
        match self.body.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                self.deadline = None;
                if let Some(bytes) = frame.data_ref() {
                    if bytes.len() > self.remaining {
                        self.finished = true;
                        return Poll::Ready(Some(Err(PortfuError::PayloadTooLarge(
                            "Request body exceeded limit".into(),
                        ))));
                    }
                    self.remaining -= bytes.len();
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(error))) => {
                self.finished = true;
                let error = match error {
                    "portfu:payload-too-large" => {
                        PortfuError::PayloadTooLarge("Request body exceeded limit".into())
                    }
                    "portfu:request-timeout" => {
                        PortfuError::RequestTimeout("Body read timeout".into())
                    }
                    _ => PortfuError::BadRequest("Failed to read request body".into()),
                };
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                self.finished = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.finished || self.body.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

impl FromRequest<Request> for RequestBody {
    type Error = PortfuError;
    fn try_from<'a>(
        request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, PortfuError>> + Send + Sync + 'a>> {
        Box::pin(async move { request.take_body() })
    }
}

/// A validated client address for request middleware.
pub fn client_ip(request: &Request) -> std::net::IpAddr {
    let peer = request
        .get::<std::net::SocketAddr>()
        .map(|peer| peer.ip())
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    request
        .get::<Arc<crate::server::Server>>()
        .filter(|server| {
            server.config.trust_proxy_headers && server.config.trusted_proxies.contains(&peer)
        })
        .and_then(|server| request.headers().get(&server.config.forwarded_ip_header))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<std::net::IpAddr>().ok())
        .unwrap_or(peer)
}

pub struct Body(pub Bytes);
impl Body {
    pub fn into_bytes(self) -> Bytes {
        self.0
    }
}
impl FromRequest<Request> for Body {
    type Error = PortfuError;
    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move { value.consume_body_bytes().await.map(Body) })
    }
}

pub struct Json<T: DeserializeOwned>(pub T);
impl<T: DeserializeOwned> Json<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}
impl<T: DeserializeOwned + Send + Sync + 'static> FromRequest<Request> for Json<T> {
    type Error = PortfuError;
    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            let bytes = value.consume_body_bytes().await?;
            serde_json::from_slice(bytes.as_ref())
                .map(Json)
                .map_err(|e| PortfuError::Parsing(format!("Failed to parse JSON body: {e}")))
        })
    }
}

pub struct Query<T: DeserializeOwned>(pub T);
impl<T: DeserializeOwned> Query<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}
impl<T: DeserializeOwned + Send + Sync + 'static> FromRequest<Request> for Query<T> {
    type Error = PortfuError;
    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            let query = value.uri().query().unwrap_or_default();
            serde_urlencoded::from_str::<T>(query)
                .map(Query)
                .map_err(|e| PortfuError::Parsing(format!("Failed to parse query string: {e}")))
        })
    }
}
