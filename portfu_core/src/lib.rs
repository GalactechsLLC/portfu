pub mod filters;
pub mod paths;
pub mod service;
pub mod data_map;

use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use http::{Response, StatusCode};
use http_body_util::Full;
use hyper::body::{Bytes};
use crate::service::{Service, ServiceRequest};

#[async_trait]
pub trait ServiceHandler {
    fn name(&self) -> &str;
    async fn handle(
        &self,
        address: &SocketAddr,
        request: &ServiceRequest,
        response: Response<Full<Bytes>>
    ) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>>;
}
impl<'a> Debug for (dyn ServiceHandler + Send + Sync + 'static) {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

struct DefaultHandler{}
#[async_trait]
impl ServiceHandler for DefaultHandler {
    fn name(&self) -> &str {
        "DefaultHandler"
    }
    async fn handle(&self, _: &SocketAddr, service_request: &ServiceRequest, response: Response<Full<Bytes>>) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>> {
        let mut response = response;
        *response.status_mut() = StatusCode::NOT_FOUND;
        *response.body_mut() = Full::new(Bytes::from(format!("Failed to find Path: {}", service_request.request.uri().path())));
        Ok(response)
    }
}

pub trait ServiceRegister {
    fn register(self, service_registry: &mut ServiceRegistry);
}

#[derive(Debug)]
pub struct ServiceRegistry {
    pub services: Vec<Arc<Service>>
}
impl<'a> ServiceRegistry {
    pub fn register(&mut self, service: Service) {
        self.services.push(Arc::new(service));
    }
}