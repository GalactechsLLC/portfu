use crate::filters::{FilterFn, FilterResult};
use crate::routes::Route;
use crate::wrappers::{WrapperFn, WrapperResult};
use crate::{ServiceData, ServiceHandler, ServiceRegister, ServiceRegistry};
use futures_util::TryStreamExt;
use http::request::Parts;
use http::{Extensions, HeaderMap, HeaderValue, Method, Request, Response, Uri};
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
use crate::editable::EditFn;

#[derive(Debug)]
pub struct ServiceBuilder {
    path: Route,
    name: Option<String>,
    editable: Option<Arc<dyn EditFn<Error=Error> + Sync + Send>>,
    filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
    handler: Option<Arc<dyn ServiceHandler + Send + Sync>>,
}
impl ServiceBuilder {
    pub fn new(path: &str) -> Self {
        Self {
            path: Route::new(path.to_string()),
            name: None,
            editable: None,
            filters: vec![],
            wrappers: vec![],
            handler: None,
        }
    }
    pub fn name<S: AsRef<str>>(self, path: S) -> Self {
        let mut s = self;
        s.name = Some(path.as_ref().to_string());
        s
    }
    pub fn filter(self, filter: Arc<dyn FilterFn + Sync + Send>) -> Self {
        let mut s = self;
        s.filters.push(filter);
        s
    }
    pub fn editable(self, editable: Arc<dyn EditFn<Error=Error> + Sync + Send>) -> Self {
        let mut s = self;
        s.editable = Some(editable);
        s
    }
    pub fn wrap(self, wrappers: Arc<dyn WrapperFn + Sync + Send>) -> Self {
        let mut s = self;
        s.wrappers.push(wrappers);
        s
    }
    pub fn handler(self, service_handler: Arc<dyn ServiceHandler + Send + Sync>) -> Self {
        let mut s = self;
        s.handler = Some(service_handler);
        s
    }
    pub fn build(self) -> Service {
        Service {
            path: Arc::new(self.path),
            editable: self.editable,
            name: self.name.unwrap_or_default(),
            filters: self.filters,
            wrappers: self.wrappers,
            handler: self.handler,
        }
    }
}

#[derive(Default)]
pub struct ServiceGroup {
    pub services: Vec<Service>,
    pub filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    pub wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
}
impl ServiceRegister for ServiceGroup {
    fn register(self, service_registry: &mut ServiceRegistry) {
        for service in self.services {
            service.register(service_registry);
        }
    }
}
impl ServiceGroup {
    pub fn service<T: ServiceRegister + Into<Service>>(mut self, service: T) -> Self {
        let mut service = service.into();
        service.filters.extend(self.filters.clone());
        service.wrappers.extend(self.wrappers.clone());
        self.services.push(service);
        self
    }
    pub fn sub_group<T: Into<ServiceGroup>>(mut self, group: T) -> Self {
        let group = group.into();
        for service in group.services {
            self = self.service(service);
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
}

#[derive(Debug)]
pub struct Service {
    pub path: Arc<Route>,
    pub name: String,
    pub editable: Option<Arc<dyn EditFn<Error=Error> + Sync + Send>>,
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
}
impl ServiceRegister for Service {
    fn register(self, service_registry: &mut ServiceRegistry) {
        service_registry.register(self)
    }
}

pub enum IncomingRequest {
    Stream(Request<Incoming>),
    Sized(Request<Full<Bytes>>),
    Consumed(Parts),
    Empty,
}
pub enum BodyType<'a> {
    Stream(&'a mut Incoming),
    Sized(&'a mut Full<Bytes>),
    Empty,
}

pub enum ConsumedBodyType {
    Stream(Incoming),
    Sized(Full<Bytes>),
    Empty,
}

impl Body for ConsumedBodyType {
    type Data = Bytes;
    type Error = String;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            ConsumedBodyType::Stream(s) => Pin::new(s)
                .poll_frame(cx)
                .map_err(|e| format!("Failed to Read from Stream Body: {e:?}")),
            ConsumedBodyType::Sized(s) => Pin::new(s).poll_frame(cx).map_err(|_| {
                String::new() //Should Never Happen, e in infallible
            }),
            ConsumedBodyType::Empty => Poll::Ready(None),
        }
    }
}

impl From<ConsumedBodyType> for reqwest::Body {
    fn from(value: ConsumedBodyType) -> reqwest::Body {
        match value {
            ConsumedBodyType::Stream(value) => {
                let body_stream = BodyStream::new(value);
                let body_stream = body_stream.map_ok(|d| d.into_data().unwrap());
                let body = StreamBody::new(body_stream);
                reqwest::Body::wrap_stream(body)
            }
            ConsumedBodyType::Sized(value) => {
                let body_stream = BodyStream::new(value);
                let body_stream = body_stream.map_ok(|d| d.into_data().unwrap());
                let body = StreamBody::new(body_stream);
                reqwest::Body::wrap_stream(body)
            }
            ConsumedBodyType::Empty => reqwest::Body::default(),
        }
    }
}
static DEFAULT_URI: Lazy<Uri> = Lazy::new(Uri::default);
impl IncomingRequest {
    pub async fn consume(self) -> Result<(Self, ConsumedBodyType), (Self, Error)> {
        match self {
            IncomingRequest::Sized(r) => {
                let (parts, body) = r.into_parts();
                let body = ConsumedBodyType::Sized(Full::new(
                    match body.collect().await {
                        Ok(b) => b.to_bytes(),
                        Err(_) => {
                            return Err((IncomingRequest::Consumed(parts), Error::new(
                                ErrorKind::InvalidData,
                                "Failed to read all Bytes from Request",
                            )));
                        }
                    }
                ));
                Ok((
                    IncomingRequest::Consumed(parts),
                    body
                ))
            }
            IncomingRequest::Stream(r) => {
                let (parts, body) = r.into_parts();
                Ok((
                    IncomingRequest::Consumed(parts),
                    ConsumedBodyType::Stream(body),
                ))
            }
            IncomingRequest::Consumed(parts) => {
                Ok((Self::Consumed(parts), ConsumedBodyType::Empty))
            }
            IncomingRequest::Empty => Ok((Self::Empty, ConsumedBodyType::Empty)),
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
    pub fn body(&mut self) -> BodyType {
        match self {
            IncomingRequest::Sized(r) => BodyType::Sized(r.body_mut()),
            IncomingRequest::Stream(r) => BodyType::Stream(r.body_mut()),
            IncomingRequest::Consumed(_) => BodyType::Empty,
            IncomingRequest::Empty => BodyType::Empty,
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
    pub fn consume(&mut self) -> Result<ConsumedBodyType, Error> {
        match replace(&mut self.request, IncomingRequest::Empty) {
            IncomingRequest::Sized(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.request, IncomingRequest::Consumed(parts));
                Ok(ConsumedBodyType::Sized(body))
            }
            IncomingRequest::Stream(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.request, IncomingRequest::Consumed(parts));
                Ok(ConsumedBodyType::Stream(body))
            }
            IncomingRequest::Consumed(parts) => {
                let _ = replace(&mut self.request, IncomingRequest::Consumed(parts));
                Ok(ConsumedBodyType::Empty)
            }
            IncomingRequest::Empty => Ok(ConsumedBodyType::Empty),
        }
    }
    pub fn set_body(&mut self, body: ConsumedBodyType) -> Result<ConsumedBodyType, Error> {
        let (parts, old_body) = match replace(&mut self.request, IncomingRequest::Empty) {
            IncomingRequest::Sized(r) => {
                let (parts, body) = r.into_parts();
                (parts, ConsumedBodyType::Sized(body))
            }
            IncomingRequest::Stream(r) => {
                let (parts, body) = r.into_parts();
                (parts, ConsumedBodyType::Stream(body))
            }
            IncomingRequest::Consumed(parts) => (parts, ConsumedBodyType::Empty),
            IncomingRequest::Empty => (Request::new(()).into_parts().0, ConsumedBodyType::Empty),
        };
        match body {
            ConsumedBodyType::Sized(s) => {
                let _ = replace(
                    &mut self.request,
                    IncomingRequest::Sized(Request::from_parts(parts, s)),
                );
            }
            ConsumedBodyType::Stream(s) => {
                let _ = replace(
                    &mut self.request,
                    IncomingRequest::Stream(Request::from_parts(parts, s)),
                );
            }
            ConsumedBodyType::Empty => {
                let _ = replace(
                    &mut self.request,
                    IncomingRequest::Sized(Request::from_parts(parts, Full::new(Bytes::new()))),
                );
            }
        }
        Ok(old_body)
    }
}
