use crate::filters::{FilterFn, FilterResult};
use crate::routes::Route;
use crate::wrappers::{WrapperFn, WrapperResult};
use crate::{ServiceData, ServiceHandler, ServiceRegister, ServiceRegistry};
use http::{Extensions, HeaderMap, HeaderValue, Method, Request, Response, Uri};
use http_body_util::Full;
use hyper::body::{Body, Bytes, Incoming, SizeHint};
use hyper::upgrade::OnUpgrade;
use std::io::Error;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;

#[derive(Debug)]
pub struct ServiceBuilder {
    path: Route,
    name: Option<String>,
    filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
    handler: Option<Arc<dyn ServiceHandler + Send + Sync>>,
}
impl ServiceBuilder {
    pub fn new(path: &str) -> Self {
        Self {
            path: Route::new(path.to_string()),
            name: None,
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
    pub filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    pub wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
    pub handler: Option<Arc<dyn ServiceHandler + Send + Sync>>,
}
impl Service {
    pub fn handles(&self, req: &Request<Incoming>) -> bool {
        self.path.matches(req.uri().path())
            && self
                .filters
                .iter()
                .cloned()
                .all(|f| f.filter(req) == FilterResult::Allow)
    }
    pub async fn handle(&self, data: &mut ServiceData) -> Result<(), Error> {
        for func in self.wrappers.iter() {
            match func.before(data).await {
                WrapperResult::Continue => {}
                WrapperResult::Return => {
                    return Ok(());
                }
            }
        }
        if let Some(handler) = self.handler.as_ref() {
            handler.handle(data).await?;
        }
        for func in self.wrappers.iter() {
            match func.after(data).await {
                WrapperResult::Continue => {}
                WrapperResult::Return => {
                    return Ok(());
                }
            };
        }
        Ok(())
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
}

pub enum BodyType<'a> {
    Stream(&'a mut Incoming),
    Sized(&'a mut Full<Bytes>),
}
impl IncomingRequest {
    pub fn uri(&self) -> &Uri {
        match &self {
            IncomingRequest::Sized(r) => r.uri(),
            IncomingRequest::Stream(r) => r.uri(),
        }
    }
    pub fn headers(&self) -> &HeaderMap<HeaderValue> {
        match &self {
            IncomingRequest::Sized(r) => r.headers(),
            IncomingRequest::Stream(r) => r.headers(),
        }
    }
    pub fn headers_mut(&mut self) -> &mut HeaderMap<HeaderValue> {
        match self {
            IncomingRequest::Sized(r) => r.headers_mut(),
            IncomingRequest::Stream(r) => r.headers_mut(),
        }
    }
    pub fn method(&self) -> &Method {
        match self {
            IncomingRequest::Sized(r) => r.method(),
            IncomingRequest::Stream(r) => r.method(),
        }
    }
    pub fn size_hint(&self) -> SizeHint {
        match &self {
            IncomingRequest::Sized(r) => r.size_hint(),
            IncomingRequest::Stream(r) => r.size_hint(),
        }
    }
    pub fn extensions(&self) -> &Extensions {
        match &self {
            IncomingRequest::Sized(r) => r.extensions(),
            IncomingRequest::Stream(r) => r.extensions(),
        }
    }
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        match self {
            IncomingRequest::Sized(r) => r.extensions_mut(),
            IncomingRequest::Stream(r) => r.extensions_mut(),
        }
    }
    pub fn body(&mut self) -> BodyType {
        match self {
            IncomingRequest::Sized(r) => BodyType::Sized(r.body_mut()),
            IncomingRequest::Stream(r) => BodyType::Stream(r.body_mut()),
        }
    }
    pub fn is_upgrade_request(&self) -> bool {
        header_contains_value(self.headers(), hyper::header::CONNECTION, "Upgrade")
            && header_contains_value(self.headers(), hyper::header::UPGRADE, "websocket")
    }
    pub fn upgrade(&mut self) -> Result<(Response<Full<Bytes>>, OnUpgrade), ProtocolError> {
        let key = self
            .headers()
            .get("Sec-WebSocket-Key")
            .ok_or(ProtocolError::MissingSecWebSocketKey)?;
        if self
            .headers()
            .get("Sec-WebSocket-Version")
            .map(|v| v.as_bytes())
            != Some(b"13")
        {
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
        self.request.extensions().get()
    }
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.request.extensions_mut().get_mut()
    }
    pub fn insert<T: Clone + Send + Sync + 'static>(&mut self, t: T) -> Option<T> {
        self.request.extensions_mut().insert(t)
    }
    pub fn remove<T: Clone + Send + Sync + 'static>(&mut self) -> Option<T> {
        self.request.extensions_mut().remove()
    }
}
