pub mod filters;
pub mod route;
pub mod service;
pub mod wrappers;

use std::fmt::{Debug, Formatter};
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use async_trait::async_trait;
use http::{Response, StatusCode};
use http_body_util::Full;
use http_body_util::BodyExt;
use hyper::body::Bytes;
use once_cell::sync::Lazy;
use serde::Deserialize;
use crate::service::{BodyType, Service, ServiceRequest};

#[async_trait]
pub trait ServiceHandler {
    fn name(&self) -> &str;
    async fn handle(
        &self,
        request: ServiceRequest,
        response: Response<Full<Bytes>>
    ) -> Result<ServiceResponse, ServiceResponse>;
}
impl<'a> Debug for (dyn ServiceHandler + Send + Sync + 'static) {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[async_trait]
impl ServiceHandler for (&'static str, &'static str) {
    fn name(&self) -> &str {
        &self.0
    }

    async fn handle(&self, request: ServiceRequest, mut response: Response<Full<Bytes>>) -> Result<ServiceResponse, ServiceResponse> {
        *response.body_mut() = Full::new(Bytes::from_static(self.1.as_bytes()));
        Ok(ServiceResponse {
            request,
            response,
        })
    }
}

pub struct ServiceResponse {
    pub request: ServiceRequest,
    pub response: Response<Full<Bytes>>
}

struct DefaultHandler{}
#[async_trait]
impl ServiceHandler for DefaultHandler {
    fn name(&self) -> &str {
        "DefaultHandler"
    }
    async fn handle(&self, request: ServiceRequest, response: Response<Full<Bytes>>) -> Result<ServiceResponse, ServiceResponse> {
        let mut response = response;
        *response.status_mut() = StatusCode::NOT_FOUND;
        *response.body_mut() = Full::new(Bytes::from(format!("Failed to find Path: {}", request.request.uri().path())));
        Ok(ServiceResponse {
            request,
            response,
        })
    }
}

pub trait ServiceRegister {
    fn register(self, service_registry: &mut ServiceRegistry);
}

pub static mut STATIC_REGISTRY: Lazy<ServiceRegistry> = Lazy::new(|| {
    ServiceRegistry{
        services: vec![]
    }
});

#[derive(Clone, Debug)]
pub struct ServiceRegistry {
    pub services: Vec<Arc<Service>>
}
impl<'a> ServiceRegistry {
    pub fn register(&mut self, service: Service) {
        self.services.push(Arc::new(service));
    }
}

#[derive(Clone)]
pub struct State<T: Send + Sync + 'static>(Arc<T>);
impl<T: Send + Sync + 'static> State<T> {
    pub fn inner(self) -> Arc<T> { self.0.clone() }
    pub async fn extract(request: &mut ServiceRequest) -> Option<State<T>> {
        request.request.extensions().get::<Arc<T>>().cloned().map(State)
    }
}

#[derive(Clone)]
pub struct Path(String);
impl Path {
    pub fn inner(self) -> String {self.0}
    pub async fn extract(request: &mut ServiceRequest, segment: &str) -> Result<Path, Error> {
        request.path.extract(request.request.uri().path(), segment)
            .and_then(|data| Some(Path(data)))
            .ok_or(
            Error::new(ErrorKind::InvalidInput, format!("Failed to parse path variable {} in path {}", segment, request.request.uri().path()))
        )
    }
}

pub struct Body<T: FromBody> {
    pub data: T
}
impl<T: FromBody> Body<T> {
    pub fn new(t: T) -> Body<T> {
        Self {
            data: t
        }
    }

    pub async fn extract(request: &mut ServiceRequest) -> Result<Body<T>, Error> {
        let mut body = request.request.body();
        T::from_body(&mut body).await.map(Body::new)
    }
}

#[async_trait::async_trait]
pub trait FromBody {
    async fn from_body(body: &mut BodyType) -> Result<Self, Error> where Self: Sized;
}

#[async_trait::async_trait]
impl<T> FromBody for T where T: for<'a> Deserialize<'a> {
    async fn from_body(body: &mut BodyType) -> Result<Self, Error> {
        let bytes = body_to_bytes(body).await?;
        serde_json::from_slice(bytes.as_ref()).map_err(|e| {
            Error::new(ErrorKind::InvalidInput, format!("Failed to parse body as JSON: {e:?}"))
        })
    }
}

async fn body_to_bytes(body: &mut BodyType<'_>) -> Result<Bytes, Error>{
    match body {
        BodyType::Sized(b) => {
            b.collect().await.map(|v| {
                v.to_bytes()
            }).map_err(|e| {
                Error::new(ErrorKind::InvalidInput, format!("Failed to read body: {e:?}"))
            })
        }
        BodyType::Stream(b) => {
            b.collect().await.map(|v| {
                v.to_bytes()
            }).map_err(|e| {
                Error::new(ErrorKind::InvalidInput, format!("Failed to read body: {e:?}"))
            })
        }
    }
}