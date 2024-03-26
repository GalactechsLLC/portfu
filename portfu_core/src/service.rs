use std::sync::Arc;
use http::{Extensions, HeaderMap, HeaderValue, Method, Request, Response, Uri};
use http_body_util::Full;
use hyper::body::{Body, Bytes, Incoming, SizeHint};
use crate::filters::{Filter, FilterFn, FilterResult};
use crate::route::Route;
use crate::{ServiceHandler, ServiceRegister, ServiceRegistry, ServiceResponse};

#[derive(Debug)]
pub struct ServiceBuilder {
    path: Route,
    name: Option<String>,
    filters: Vec<Arc<Filter>>,
    handler: Option<Arc<dyn ServiceHandler + Send + Sync>>
}
impl<'a> ServiceBuilder {
    pub fn new(path: &str) -> Self {
        Self {
            path: Route::parse(path.to_string()),
            name: None,
            filters: vec![],
            handler: None
        }
    }
    pub fn name<S: AsRef<str>>(self, path: S) -> Self {
        let mut s = self;
        s.name = Some(path.as_ref().to_string());
        s
    }
    pub fn filter(self, filter: Arc<Filter>) -> Self {
        let mut s = self;
        s.filters.push(filter);
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
            name: self.name.expect("Service Name Not Set"),
            filters: self.filters,
            handler: self.handler,
        }
    }
}

#[derive(Debug)]
pub struct Service {
    pub path: Arc<Route>,
    pub name: String,
    filters: Vec<Arc<Filter>>,
    handler: Option<Arc<dyn ServiceHandler + Send + Sync>>,
}
impl Service {
    pub fn handles(&self, req: &Request<Incoming>) -> bool {
        self.path.matches(req.uri().path()) && self.filters.iter().cloned().all(|f| f.filter(req) == FilterResult::Allow)
    }
    pub async fn handle(&self, request: ServiceRequest, response: Response<Full<Bytes>>) -> Result<ServiceResponse, ServiceResponse> {
        let mut response = ServiceResponse {
            request,
            response
        };
        println!("Handled by {:?}", self.name());
        if let Some(handler) = self.handler.as_ref() {
            response = handler.handle(response.request, response.response).await?;
        }
        Ok(response)
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
    pub fn method(&self) -> &Method {
        match &self {
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