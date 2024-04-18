pub mod files;
pub mod filters;
pub mod routes;
pub mod service;
pub mod sockets;
pub mod task;
pub mod wrappers;

use crate::service::{BodyType, Service, ServiceRequest};
use async_trait::async_trait;
use http::{HeaderMap, Response, StatusCode};
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::Bytes;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::fmt::{Debug, Formatter};
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::sync::Arc;

#[async_trait]
pub trait ServiceHandler {
    fn name(&self) -> &str;
    async fn handle(&self, data: &mut ServiceData) -> Result<(), Error>;
}
impl Debug for (dyn ServiceHandler + Send + Sync + 'static) {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[async_trait]
impl ServiceHandler for (&'static str, &'static str) {
    fn name(&self) -> &str {
        self.0
    }

    async fn handle(&self, data: &mut ServiceData) -> Result<(), Error> {
        *data.response.body_mut() = Full::new(Bytes::from_static(self.1.as_bytes()));
        Ok(())
    }
}

#[async_trait]
impl ServiceHandler for (String, String) {
    fn name(&self) -> &str {
        &self.0
    }

    async fn handle(&self, data: &mut ServiceData) -> Result<(), Error> {
        *data.response.body_mut() = Full::new(Bytes::from(self.1.clone()));
        Ok(())
    }
}

#[async_trait]
impl ServiceHandler for (&'static str, &'static [u8]) {
    fn name(&self) -> &str {
        self.0
    }

    async fn handle(&self, data: &mut ServiceData) -> Result<(), Error> {
        *data.response.body_mut() = Full::new(Bytes::from_static(self.1));
        Ok(())
    }
}

pub struct ServiceData {
    pub request: ServiceRequest,
    pub response: Response<Full<Bytes>>,
}
impl ServiceData {
    pub fn get_best_guess_public_ip(&self, address: &SocketAddr) -> String {
        if let Some(real_ip) = self.request.request.headers().get("x-real-ip") {
            format!("{:?}", real_ip)
        } else if let Some(forwards) = self.request.request.headers().get("x-forwarded-for") {
            format!("{:?}", forwards)
        } else {
            address.to_string()
        }
    }
}

struct DefaultHandler {}
#[async_trait]
impl ServiceHandler for DefaultHandler {
    fn name(&self) -> &str {
        "DefaultHandler"
    }
    async fn handle(&self, data: &mut ServiceData) -> Result<(), Error> {
        *data.response.status_mut() = StatusCode::NOT_FOUND;
        *data.response.body_mut() = Full::new(Bytes::from(format!(
            "Failed to find Path: {}",
            data.request.request.uri().path()
        )));
        Ok(())
    }
}

pub trait ServiceRegister {
    fn register(self, service_registry: &mut ServiceRegistry);
}

pub static mut STATIC_REGISTRY: Lazy<ServiceRegistry> =
    Lazy::new(|| ServiceRegistry { services: vec![] });

#[derive(Clone, Debug, Default)]
pub struct ServiceRegistry {
    pub services: Vec<Arc<Service>>,
}
impl ServiceRegistry {
    pub fn register(&mut self, service: Service) {
        self.services.push(Arc::new(service));
    }
}

#[async_trait]
pub trait FromRequest<'a>
where
    Self: Sized,
{
    async fn from_request(
        request: &'a mut ServiceRequest,
        var_name: &'a str,
    ) -> Result<Self, Error>;
}

#[derive(Clone)]
pub struct State<T: Send + Sync + 'static>(pub Arc<T>);
impl<T: Send + Sync + 'static> State<T> {
    pub fn inner(&self) -> Arc<T> {
        self.0.clone()
    }
}
impl<T: Send + Sync + 'static> AsRef<T> for State<T> {
    fn as_ref(&self) -> &T {
        self.0.as_ref()
    }
}
#[async_trait]
impl<'a, T: Send + Sync + 'static> FromRequest<'a> for State<T> {
    async fn from_request(request: &'a mut ServiceRequest, _: &'a str) -> Result<Self, Error> {
        request
            .request
            .extensions()
            .get::<Arc<T>>()
            .cloned()
            .map(State)
            .ok_or(Error::new(ErrorKind::NotFound, "Failed to find State"))
    }
}
#[async_trait]
impl<'a> FromRequest<'a> for &'a mut HeaderMap {
    async fn from_request(request: &'a mut ServiceRequest, _: &'a str) -> Result<Self, Error> {
        Ok(request.request.headers_mut())
    }
}

#[async_trait]
impl<'a> FromRequest<'a> for SocketAddr {
    async fn from_request(request: &'a mut ServiceRequest, _: &'a str) -> Result<Self, Error> {
        request
            .get()
            .copied()
            .ok_or(Error::new(ErrorKind::NotFound, "Failed to find SocketAddr"))
    }
}

#[derive(Clone)]
pub struct Path(String);
impl Path {
    pub fn inner(self) -> String {
        self.0
    }
}
#[async_trait]
impl<'a> FromRequest<'a> for Path {
    async fn from_request(
        request: &'a mut ServiceRequest,
        var_name: &'a str,
    ) -> Result<Self, Error> {
        request
            .path
            .extract(request.request.uri().path(), var_name)
            .map(Path)
            .ok_or(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "Failed to parse path variable {} in path {}",
                    var_name,
                    request.request.uri().path()
                ),
            ))
    }
}

pub struct Body<T: FromBody>(T);
impl<T: FromBody> Body<T> {
    pub fn inner(self) -> T {
        self.0
    }
}
impl<T: FromBody> AsRef<T> for Body<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}
impl<T: FromBody> AsMut<T> for Body<T> {
    fn as_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
#[async_trait]
impl<'a, T: FromBody> FromRequest<'a> for Body<T> {
    async fn from_request(request: &'a mut ServiceRequest, _: &'a str) -> Result<Self, Error> {
        let mut body = request.request.body();
        T::from_body(&mut body).await.map(Body)
    }
}

#[async_trait::async_trait]
pub trait FromBody {
    async fn from_body(body: &mut BodyType) -> Result<Self, Error>
    where
        Self: Sized;
}

#[async_trait::async_trait]
impl FromBody for String {
    async fn from_body(body: &mut BodyType) -> Result<Self, Error> {
        let bytes = body_to_bytes(body).await?;
        Ok(String::from_utf8_lossy(bytes.as_ref()).to_string())
    }
}

pub struct Json<T: for<'a> Deserialize<'a>>(T);
impl<T: for<'a> Deserialize<'a>> Json<T> {
    pub fn inner(self) -> T {
        self.0
    }
}

#[async_trait::async_trait]
impl<T> FromBody for Json<T>
where
    T: for<'a> Deserialize<'a>,
{
    async fn from_body(body: &mut BodyType) -> Result<Self, Error> {
        let bytes = body_to_bytes(body).await?;
        serde_json::from_slice(bytes.as_ref())
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("Failed to parse body as JSON: {e:?}"),
                )
            })
            .map(Json)
    }
}

macro_rules! from_body {
    ($int:ident) => {
        #[async_trait::async_trait]
        impl FromBody for $int {
            async fn from_body(body: &mut BodyType) -> Result<Self, Error> {
                let bytes = body_to_bytes(body).await?;
                let as_str = String::from_utf8_lossy(bytes.as_ref());
                as_str.parse().map_err(|e| {
                    Error::new(
                        ErrorKind::InvalidInput,
                        format!("Failed to parse body as $int: {e:?}"),
                    )
                })
            }
        }
    };
}

from_body!(u8);
from_body!(u16);
from_body!(u32);
from_body!(u64);
from_body!(u128);
from_body!(i8);
from_body!(i16);
from_body!(i32);
from_body!(i64);
from_body!(i128);

async fn body_to_bytes(body: &mut BodyType<'_>) -> Result<Bytes, Error> {
    match body {
        BodyType::Sized(b) => b.collect().await.map(|v| v.to_bytes()).map_err(|e| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("Failed to read body: {e:?}"),
            )
        }),
        BodyType::Stream(b) => b.collect().await.map(|v| v.to_bytes()).map_err(|e| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("Failed to read body: {e:?}"),
            )
        }),
    }
}
