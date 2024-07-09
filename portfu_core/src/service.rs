use crate::filters::{FilterFn, FilterResult};
use crate::routes::Route;
use crate::wrappers::{WrapperFn, WrapperResult};
use crate::{IntoStreamBody, StreamingBody, ServiceData, ServiceHandler, ServiceRegister, ServiceRegistry};
use futures_util::TryStreamExt;
use http::{Extensions, HeaderMap, HeaderValue, Method, Request, request, Response, response, StatusCode, Uri};
use http_body::Frame;
use http_body_util::{BodyExt, BodyStream, Empty, Full, StreamBody};
use hyper::body::{Body, Bytes, Incoming, SizeHint};
use hyper::upgrade::OnUpgrade;
use once_cell::sync::Lazy;
use std::io::{Error, ErrorKind};
use std::mem::replace;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use uuid::Uuid;
use crate::task::{Task, TaskFn};

#[derive(Debug)]
pub struct ServiceBuilder {
    path: Route,
    name: Option<String>,
    shared_state: Extensions,
    filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
    tasks: Vec<Arc<dyn TaskFn + Send + Sync>>,
    handler: Option<Arc<dyn ServiceHandler + Send + Sync>>,
}
impl ServiceBuilder {
    pub fn new(path: &str) -> Self {
        Self {
            path: Route::new(path.to_string()),
            name: None,
            filters: vec![],
            wrappers: vec![],
            tasks: vec![],
            shared_state: Default::default(),
            handler: None,
        }
    }
    pub fn name<S: AsRef<str>>(mut self, path: S) -> Self {
        self.name = Some(path.as_ref().to_string());
        self
    }
    pub fn shared_state<T: Send + Sync + 'static>(mut self, shared_state: T) -> Self {
        self.shared_state.insert(Arc::new(shared_state));
        self
    }
    pub fn extend_state(mut self, shared_state: Extensions) -> Self {
        self.shared_state.extend(shared_state);
        self
    }
    pub fn filter(mut self, filter: Arc<dyn FilterFn + Sync + Send>) -> Self {
        self.filters.push(filter);
        self
    }
    pub fn task(mut self, task: Arc<dyn TaskFn + Sync + Send>) -> Self {
        self.tasks.push(task);
        self
    }
    pub fn wrap(mut self, wrappers: Arc<dyn WrapperFn + Sync + Send>) -> Self {
        self.wrappers.push(wrappers);
        self
    }
    pub fn handler(mut self, service_handler: Arc<dyn ServiceHandler + Send + Sync>) -> Self {
        self.handler = Some(service_handler);
        self
    }
    pub fn build(self) -> Service {
        Service {
            path: Arc::new(self.path),
            name: self.name.unwrap_or_default(),
            uuid: Uuid::new_v4(),
            shared_state: self.shared_state,
            filters: self.filters,
            wrappers: self.wrappers,
            handler: self.handler,
        }
    }
}

#[derive(Default)]
pub struct ServiceGroup {
    pub services: Vec<Service>,
    pub shared_state: Extensions,
    pub filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    pub wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
    pub tasks: Vec<Arc<dyn TaskFn + Sync + Send>>,
}
impl ServiceRegister for ServiceGroup {
    fn register(self, service_registry: &mut ServiceRegistry, shared_state: Extensions) {
        for service in self.services {
            service.register(service_registry, shared_state.clone());
        }
        for task in self.tasks {
            service_registry.tasks.push(Arc::new(Task {
                name: task.name().to_string(),
                task_fn: task,
            }));
        }
    }
}
impl ServiceGroup {
    pub fn service<T: ServiceRegister + Into<Service>>(mut self, service: T) -> Self {
        let mut service = service.into();
        service.filters.extend(self.filters.clone());
        service.wrappers.extend(self.wrappers.clone());
        service.shared_state.extend(self.shared_state.clone());
        self.services.push(service);
        self
    }
    pub fn shared_state<T: Send + Sync + 'static>(mut self, shared_state: T) -> Self {
        self.shared_state.insert(Arc::new(shared_state));
        self
    }
    pub fn sub_group<T: Into<ServiceGroup>>(mut self, group: T) -> Self {
        let group = group.into();
        self.shared_state.extend(group.shared_state.clone());
        for service in group.services {
            self = self.service(service);
        }
        for task in group.tasks {
            self = self.task(task);
        }
        self
    }
    pub fn filter(mut self, filter: Arc<dyn FilterFn + Sync + Send>) -> Self {
        self.filters.push(filter);
        self
    }
    pub fn wrap(mut self, wrappers: Arc<dyn WrapperFn + Sync + Send>) -> Self {
        self.wrappers.push(wrappers);
        self
    }
    pub fn task(mut self, task: Arc<dyn TaskFn + Sync + Send>) -> Self {
        self.tasks.push(task);
        self
    }
}

#[derive(Debug)]
pub struct Service {
    pub path: Arc<Route>,
    pub name: String,
    pub uuid: Uuid,
    pub shared_state: Extensions,
    pub filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    pub wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
    pub handler: Option<Arc<dyn ServiceHandler + Send + Sync>>,
}
impl Service {
    pub async fn handles(&self, req: &Request<Incoming>) -> bool {
        if self.path.matches(req.uri().path()) {
            for f in self.filters.iter() {
                if f.filter(req).await != FilterResult::Allow {
                    return false;
                }
            }
            true
        } else {
            false
        }
    }
    pub async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, (ServiceData, Error)> {
        for func in self.wrappers.iter() {
            match func.before(&mut data).await {
                WrapperResult::Continue => {}
                WrapperResult::Return => {
                    return Ok(data);
                }
            }
        }
        if let Some(handler) = self.handler.as_ref() {
            data = handler.handle(data).await?;
        }
        for func in self.wrappers.iter() {
            match func.after(&mut data).await {
                WrapperResult::Continue => {}
                WrapperResult::Return => {
                    return Ok(data);
                }
            };
        }
        Ok(data)
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub fn uuid(&self) -> &Uuid {
        &self.uuid
    }
}
impl ServiceRegister for Service {
    fn register(mut self, service_registry: &mut ServiceRegistry, shared_state: Extensions) {
        self.shared_state.extend(shared_state);
        service_registry.register(self)
    }
}

pub enum IncomingRequest {
    Stream(Request<StreamingBody>),
    Sized(Request<Full<Bytes>>),
    Consumed(request::Parts),
    Empty,
}

pub enum OutgoingResponse {
    Stream(Response<StreamingBody>),
    Sized(Response<Full<Bytes>>),
    Consumed(response::Parts),
    Empty(Response<()>),
}
pub enum RefBodyType<'a> {
    Stream(&'a mut StreamingBody),
    Sized(&'a mut Full<Bytes>),
    Empty,
}

pub enum BodyType {
    Stream(StreamingBody),
    Sized(Full<Bytes>),
    Empty,
}

impl Body for BodyType {
    type Data = Bytes;
    type Error = String;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            BodyType::Stream(s) => Pin::new(s)
                .poll_frame(cx)
                .map_err(|e| format!("Failed to Read from Stream Body: {e:?}")),
            BodyType::Sized(s) => Pin::new(s).poll_frame(cx).map_err(|_| {
                String::new() //Should Never Happen, e in infallible
            }),
            BodyType::Empty => Poll::Ready(None),
        }
    }
}

impl From<BodyType> for reqwest::Body {
    fn from(value: BodyType) -> reqwest::Body {
        match value {
            BodyType::Stream(value) => {
                let body_stream = BodyStream::new(value);
                let body_stream = body_stream.map_ok(|d| d.into_data().unwrap());
                let body = StreamBody::new(body_stream);
                reqwest::Body::wrap_stream(body)
            }
            BodyType::Sized(value) => {
                let body_stream = BodyStream::new(value);
                let body_stream = body_stream.map_ok(|d| d.into_data().unwrap());
                let body = StreamBody::new(body_stream);
                reqwest::Body::wrap_stream(body)
            }
            BodyType::Empty => reqwest::Body::default(),
        }
    }
}
static DEFAULT_URI: Lazy<Uri> = Lazy::new(Uri::default);
impl IncomingRequest {
    pub async fn consume(self) -> Result<(Self, BodyType), (Self, Error)> {
        match self {
            IncomingRequest::Sized(r) => {
                let (parts, body) = r.into_parts();
                let body = BodyType::Sized(Full::new(match body.collect().await {
                    Ok(b) => b.to_bytes(),
                    Err(_) => {
                        return Err((
                            IncomingRequest::Consumed(parts),
                            Error::new(
                                ErrorKind::InvalidData,
                                "Failed to read all Bytes from Request",
                            ),
                        ));
                    }
                }));
                Ok((IncomingRequest::Consumed(parts), body))
            }
            IncomingRequest::Stream(r) => {
                let (parts, body) = r.into_parts();
                Ok((
                    IncomingRequest::Consumed(parts),
                    BodyType::Stream(body),
                ))
            }
            IncomingRequest::Consumed(parts) => {
                Ok((Self::Consumed(parts), BodyType::Empty))
            }
            IncomingRequest::Empty => Ok((Self::Empty, BodyType::Empty)),
        }
    }
    pub fn uri(&self) -> &Uri {
        match &self {
            IncomingRequest::Sized(r) => r.uri(),
            IncomingRequest::Stream(r) => r.uri(),
            IncomingRequest::Consumed(r) => &r.uri,
            IncomingRequest::Empty => &DEFAULT_URI,
        }
    }
    pub fn headers(&self) -> Option<&HeaderMap<HeaderValue>> {
        match &self {
            IncomingRequest::Sized(r) => Some(r.headers()),
            IncomingRequest::Stream(r) => Some(r.headers()),
            IncomingRequest::Consumed(r) => Some(&r.headers),
            IncomingRequest::Empty => None,
        }
    }
    pub fn headers_mut(&mut self) -> Option<&mut HeaderMap<HeaderValue>> {
        match self {
            IncomingRequest::Sized(r) => Some(r.headers_mut()),
            IncomingRequest::Stream(r) => Some(r.headers_mut()),
            IncomingRequest::Consumed(r) => Some(&mut r.headers),
            IncomingRequest::Empty => None,
        }
    }
    pub fn method(&self) -> &Method {
        match self {
            IncomingRequest::Sized(r) => r.method(),
            IncomingRequest::Stream(r) => r.method(),
            IncomingRequest::Consumed(r) => &r.method,
            IncomingRequest::Empty => &Method::GET,
        }
    }
    pub fn size_hint(&self) -> SizeHint {
        match &self {
            IncomingRequest::Sized(r) => r.size_hint(),
            IncomingRequest::Stream(r) => r.size_hint(),
            IncomingRequest::Consumed(_) => SizeHint::with_exact(0),
            IncomingRequest::Empty => SizeHint::with_exact(0),
        }
    }
    pub fn extensions(&self) -> Option<&Extensions> {
        match &self {
            IncomingRequest::Sized(r) => Some(r.extensions()),
            IncomingRequest::Stream(r) => Some(r.extensions()),
            IncomingRequest::Consumed(r) => Some(&r.extensions),
            IncomingRequest::Empty => None,
        }
    }
    pub fn extensions_mut(&mut self) -> Option<&mut Extensions> {
        match self {
            IncomingRequest::Sized(r) => Some(r.extensions_mut()),
            IncomingRequest::Stream(r) => Some(r.extensions_mut()),
            IncomingRequest::Consumed(r) => Some(&mut r.extensions),
            IncomingRequest::Empty => None,
        }
    }
    pub fn body(&mut self) -> RefBodyType {
        match self {
            IncomingRequest::Sized(r) => RefBodyType::Sized(r.body_mut()),
            IncomingRequest::Stream(r) => RefBodyType::Stream(r.body_mut()),
            IncomingRequest::Consumed(_) => RefBodyType::Empty,
            IncomingRequest::Empty => RefBodyType::Empty,
        }
    }
    pub fn is_upgrade_request(&self) -> bool {
        if let Some(headers) = self.headers() {
            header_contains_value(headers, hyper::header::CONNECTION, "Upgrade")
                && header_contains_value(headers, hyper::header::UPGRADE, "websocket")
        } else {
            false
        }
    }
    pub fn upgrade(&mut self) -> Result<(Response<Full<Bytes>>, OnUpgrade), ProtocolError> {
        if let Some(headers) = self.headers() {
            let key = headers
                .get("Sec-WebSocket-Key")
                .ok_or(ProtocolError::MissingSecWebSocketKey)?;
            if headers.get("Sec-WebSocket-Version").map(|v| v.as_bytes()) != Some(b"13") {
                return Err(ProtocolError::MissingSecWebSocketVersionHeader);
            }
            let response = Response::builder()
                .status(hyper::StatusCode::SWITCHING_PROTOCOLS)
                .header(hyper::header::CONNECTION, "upgrade")
                .header(hyper::header::UPGRADE, "websocket")
                .header("Sec-WebSocket-Accept", &derive_accept_key(key.as_bytes()))
                .body(Full::<Bytes>::from("switching to websocket protocol"))
                .expect("bug: failed to build response");
            match self {
                IncomingRequest::Stream(request) => Ok((response, hyper::upgrade::on(request))),
                IncomingRequest::Sized(request) => Ok((response, hyper::upgrade::on(request))),
                IncomingRequest::Consumed(parts) => Ok((
                    response,
                    hyper::upgrade::on(Request::<Empty<()>>::from_parts(
                        parts.clone(),
                        Empty::default(),
                    )),
                )),
                IncomingRequest::Empty => Err(ProtocolError::InvalidCloseSequence), //maye a different error? Should not ever happen
            }
        } else {
            Err(ProtocolError::MissingSecWebSocketKey)
        }
    }
}

impl OutgoingResponse {

    pub fn headers_mut(&mut self) -> &mut HeaderMap{
        match self {
            OutgoingResponse::Stream(r) => r.headers_mut(),
            OutgoingResponse::Sized(r) => r.headers_mut(),
            OutgoingResponse::Consumed(r) => &mut r.headers,
            OutgoingResponse::Empty(r) => r.headers_mut()
        }
    }

    pub fn status_mut(&mut self) -> &mut StatusCode{
        match self {
            OutgoingResponse::Stream(r) => r.status_mut(),
            OutgoingResponse::Sized(r) => r.status_mut(),
            OutgoingResponse::Consumed(r) => &mut r.status,
            OutgoingResponse::Empty(r) => r.status_mut()
        }
    }

    pub fn status(&self) -> StatusCode{
        match self {
            OutgoingResponse::Stream(r) => r.status(),
            OutgoingResponse::Sized(r) => r.status(),
            OutgoingResponse::Consumed(r) => r.status,
            OutgoingResponse::Empty(r) => r.status()
        }
    }

    pub fn set_body(&mut self, body: BodyType) {
        fn handle_body(parts: response::Parts, body_type: BodyType) -> OutgoingResponse {
            match body_type {
                BodyType::Stream(b) => {
                    OutgoingResponse::Stream(Response::from_parts(parts, b))
                }
                BodyType::Sized(b) => {
                    OutgoingResponse::Sized(Response::from_parts(parts, b))
                }
                BodyType::Empty => {
                    OutgoingResponse::Empty(Response::from_parts(parts, ()))
                }
            }
        }
        match replace(self, OutgoingResponse::Empty(Response::new(()))) {
            OutgoingResponse::Stream(r) => {
                let (parts, _) = r.into_parts();
                let _ = replace(self, handle_body(parts, body));
            }
            OutgoingResponse::Sized(r) => {
                let (parts, _) = r.into_parts();
                let _ = replace(self, handle_body(parts, body));
            }
            OutgoingResponse::Consumed(parts) => {
                let _ = replace(self, handle_body(parts, body));
            }
            OutgoingResponse::Empty(r)=> {
                let (parts, _) = r.into_parts();
                let _ = replace(self, handle_body(parts, body));
            }
        }
    }
    pub fn consume(self) -> (Self, BodyType) {
        match self {
            OutgoingResponse::Sized(r) => {
                let (parts, body) = r.into_parts();
                let body = BodyType::Sized(body);
                (OutgoingResponse::Consumed(parts), body)
            }
            OutgoingResponse::Stream(r) => {
                let (parts, body) = r.into_parts();
                (
                    OutgoingResponse::Consumed(parts),
                    BodyType::Stream(body),
                )
            }
            OutgoingResponse::Consumed(parts) => {
                (Self::Consumed(parts), BodyType::Empty)
            }
            OutgoingResponse::Empty(r) => (Self::Empty(r), BodyType::Empty)
        }
    }
}

fn header_contains_value(
    headers: &HeaderMap,
    header: impl hyper::header::AsHeaderName,
    value: impl AsRef<str>,
) -> bool {
    let value = value.as_ref();
    for header in headers.get_all(header) {
        if header
            .to_str()
            .unwrap_or_default()
            .split(',')
            .any(|x| x.trim().eq_ignore_ascii_case(value))
        {
            return true;
        }
    }
    false
}

pub struct ServiceRequest {
    pub request: IncomingRequest,
    pub path: Arc<Route>,
}
impl ServiceRequest {
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        if let Some(ext) = self.request.extensions() {
            ext.get()
        } else {
            None
        }
    }
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        if let Some(ext) = self.request.extensions_mut() {
            ext.get_mut()
        } else {
            None
        }
    }
    pub fn insert<T: Clone + Send + Sync + 'static>(&mut self, t: T) -> Option<T> {
        if let Some(ext) = self.request.extensions_mut() {
            ext.insert(t)
        } else {
            None
        }
    }
    pub fn remove<T: Clone + Send + Sync + 'static>(&mut self) -> Option<T> {
        if let Some(ext) = self.request.extensions_mut() {
            ext.remove()
        } else {
            None
        }
    }
}

pub struct ServiceResponse {
    response: OutgoingResponse,
}
impl From<ServiceResponse> for Response<StreamingBody> {
    fn from(value: ServiceResponse) -> Self {
        match value.response {
            OutgoingResponse::Stream(r) => {
                let (parts, body) = r.into_parts();
                Response::from_parts(parts, body)
            },
            OutgoingResponse::Sized(r) => {
                let (parts, body) = r.into_parts();
                Response::from_parts(parts, body.stream_body())
            },
            OutgoingResponse::Consumed(parts) => {
                Response::from_parts(parts, Full::new(Bytes::new()).stream_body())
            },
            OutgoingResponse::Empty(r) => {
                let (parts, _) = r.into_parts();
                Response::from_parts(parts, Full::new(Bytes::new()).stream_body())
            },
        }
    }
}
impl Default for ServiceResponse {
    fn default() -> Self {
        Self::new()
    }
}
impl ServiceResponse {
    pub fn new() -> Self {
        Self {
            response: OutgoingResponse::Empty(Response::new(())),
        }
    }
    pub fn set_response(&mut self, outgoing: OutgoingResponse) {
        self.response = outgoing;
    }
    pub fn set_body(&mut self, body: BodyType) {
        self.response.set_body(body)
    }
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.response.headers_mut()
    }
    pub fn status_mut(&mut self) -> &mut StatusCode {
        self.response.status_mut()
    }
    pub fn status(&self) -> StatusCode {
        self.response.status()
    }
}

pub trait MutBody {
    fn consume(&mut self) -> BodyType;
    fn set_body(&mut self, body: BodyType);
}

impl MutBody for ServiceRequest {
    fn consume(&mut self) -> BodyType {
        match replace(&mut self.request, IncomingRequest::Empty) {
            IncomingRequest::Sized(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.request, IncomingRequest::Consumed(parts));
                BodyType::Sized(body)
            }
            IncomingRequest::Stream(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.request, IncomingRequest::Consumed(parts));
                BodyType::Stream(body)
            }
            IncomingRequest::Consumed(parts) => {
                let _ = replace(&mut self.request, IncomingRequest::Consumed(parts));
                BodyType::Empty
            }
            IncomingRequest::Empty => BodyType::Empty,
        }
    }
    fn set_body(&mut self, body: BodyType) {
        let (parts, _) = match replace(&mut self.request, IncomingRequest::Empty) {
            IncomingRequest::Sized(r) => {
                let (parts, body) = r.into_parts();
                (parts, BodyType::Sized(body))
            }
            IncomingRequest::Stream(r) => {
                let (parts, body) = r.into_parts();
                (parts, BodyType::Stream(body))
            }
            IncomingRequest::Consumed(parts) => (parts, BodyType::Empty),
            IncomingRequest::Empty => (Request::new(()).into_parts().0, BodyType::Empty),
        };
        match body {
            BodyType::Sized(s) => {
                let _ = replace(
                    &mut self.request,
                    IncomingRequest::Sized(Request::from_parts(parts, s)),
                );
            }
            BodyType::Stream(s) => {
                let _ = replace(
                    &mut self.request,
                    IncomingRequest::Stream(Request::from_parts(parts, s)),
                );
            }
            BodyType::Empty => {
                let _ = replace(
                    &mut self.request,
                    IncomingRequest::Sized(Request::from_parts(parts, Full::new(Bytes::new()))),
                );
            }
        }
    }
}

impl MutBody for ServiceResponse {
    fn consume(&mut self) -> BodyType {
        match replace(&mut self.response, OutgoingResponse::Empty(Response::new(()))) {
            OutgoingResponse::Sized(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.response, OutgoingResponse::Consumed(parts));
                BodyType::Sized(body)
            }
            OutgoingResponse::Stream(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.response, OutgoingResponse::Consumed(parts));
                BodyType::Stream(body)
            }
            OutgoingResponse::Consumed(parts) => {
                let _ = replace(&mut self.response, OutgoingResponse::Consumed(parts));
                BodyType::Empty
            }
            OutgoingResponse::Empty(r) => {
                let (parts, _) = r.into_parts();
                let _ = replace(&mut self.response, OutgoingResponse::Consumed(parts));
                BodyType::Empty
            }
        }
    }
    fn set_body(&mut self, body: BodyType) {
        let (parts, _) = match replace(&mut self.response, OutgoingResponse::Empty(Response::new(()))) {
            OutgoingResponse::Sized(r) => {
                let (parts, body) = r.into_parts();
                (parts, BodyType::Sized(body))
            }
            OutgoingResponse::Stream(r) => {
                let (parts, body) = r.into_parts();
                (parts, BodyType::Stream(body))
            }
            OutgoingResponse::Consumed(parts) => (parts, BodyType::Empty),
            OutgoingResponse::Empty(r) => (r.into_parts().0, BodyType::Empty),
        };
        match body {
            BodyType::Sized(s) => {
                let _ = replace(
                    &mut self.response,
                    OutgoingResponse::Sized(Response::from_parts(parts, s)),
                );
            }
            BodyType::Stream(s) => {
                let _ = replace(
                    &mut self.response,
                    OutgoingResponse::Stream(Response::from_parts(parts, s)),
                );
            }
            BodyType::Empty => {
                let _ = replace(
                    &mut self.response,
                    OutgoingResponse::Sized(Response::from_parts(parts, Full::new(Bytes::new()))),
                );
            }
        }
    }
}