use std::net::SocketAddr;
use std::sync::Arc;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use crate::data_map::DynMap;
use crate::filters::Filter;
use crate::paths::Path;
use crate::ServiceHandler;

#[derive(Debug)]
pub struct ServiceBuilder {
    path: Path,
    name: Option<String>,
    filters: Vec<Arc<Filter>>,
    handler: Option<Arc<dyn ServiceHandler + Send + Sync>>
}
impl<'a> ServiceBuilder {
    pub fn new(path: &str) -> Self {
        Self {
            path: Path::parse(path.to_string()),
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
    pub path: Arc<Path>,
    pub name: String,
    filters: Vec<Arc<Filter>>,
    handler: Option<Arc<dyn ServiceHandler + Send + Sync>>,
}
impl Service {
    pub fn handles(&self, req: &Request<Incoming>) -> bool {
        self.path.matches(req.uri().path())
    }
    pub async fn handle(&self, address: &SocketAddr, req: &ServiceRequest, response: Response<Full<Bytes>>) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>> {
        let mut response = response;
        println!("Handled by {:?}", self.name());
        for filter in self.filters.iter() {
            response = filter.handle(address, req, response).await?;
        }
        if let Some(handler) = self.handler.as_ref() {
            response = handler.handle(address, req, response).await?;
        }
        Ok(response)
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
}

pub struct ServiceRequest {
    pub request: Request<Incoming>,
    pub path: Arc<Path>,
    pub dyn_map: DynMap
}