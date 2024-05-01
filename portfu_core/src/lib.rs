pub mod files;
pub mod filters;
pub mod routes;
pub mod service;
pub mod sockets;
pub mod task;
pub mod wrappers;

use crate::service::{BodyType, IncomingRequest, Service, ServiceRequest};
use async_trait::async_trait;
use http::{Response, StatusCode};
use http_body_util::Full;
use http_body_util::{BodyExt, BodyStream, StreamBody};
use hyper::body::Bytes;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::fmt::{Debug, Formatter};
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

#[async_trait]
pub trait ServiceHandler {
    fn name(&self) -> &str;
    async fn handle(&self, data: ServiceData) -> Result<ServiceData, Error>;
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

    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, Error> {
        *data.response.body_mut() = Bytes::from_static(self.1.as_bytes()).stream_body();
        Ok(data)
    }
}

#[async_trait]
impl ServiceHandler for (String, String) {
    fn name(&self) -> &str {
        &self.0
    }

    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, Error> {
        *data.response.body_mut() = Bytes::from(self.1.clone()).stream_body();
        Ok(data)
    }
}

#[async_trait]
impl ServiceHandler for (&'static str, &'static [u8]) {
    fn name(&self) -> &str {
        self.0
    }

    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, Error> {
        *data.response.body_mut() = Bytes::from_static(self.1).stream_body();
        Ok(data)
    }
}
pub type BoxedBody =
    Box<dyn hyper::body::Body<Data = Bytes, Error = IntoStreamError> + Send + Sync + 'static>;
pub type ServiceBody = StreamBody<BodyStream<Pin<BoxedBody>>>;
pub type ServiceResponse = Response<ServiceBody>;

type IntoStreamError = &'static str;

pub trait IntoStreamBody {
    type Data;
    type Error;
    fn stream_body(self) -> ServiceBody;
}

impl IntoStreamBody for Bytes {
    type Data = Bytes;
    type Error = IntoStreamError;
    fn stream_body(self) -> ServiceBody {
        StreamBody::new(BodyStream::new(Box::pin(
            Full::new(self).map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl IntoStreamBody for String {
    type Data = Bytes;
    type Error = IntoStreamError;
    fn stream_body(self) -> ServiceBody {
        StreamBody::new(BodyStream::new(Box::pin(
            Full::new(Bytes::from(self)).map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl<'a> IntoStreamBody for &'a str {
    type Data = Bytes;
    type Error = IntoStreamError;
    fn stream_body(self) -> ServiceBody {
        StreamBody::new(BodyStream::new(Box::pin(
            Full::new(Bytes::from(self.to_string()))
                .map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl IntoStreamBody for Full<Bytes> {
    type Data = Bytes;
    type Error = IntoStreamError;

    fn stream_body(self) -> ServiceBody {
        StreamBody::new(BodyStream::new(Box::pin(
            self.map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

pub struct ServiceData {
    pub request: ServiceRequest,
    pub response: ServiceResponse,
}
impl ServiceData {
    pub fn get_best_guess_public_ip(&self, address: &SocketAddr) -> String {
        if let Some(headers) = self.request.request.headers() {
            if let Some(real_ip) = headers.get("x-real-ip") {
                format!("{:?}", real_ip)
            } else if let Some(forwards) = headers.get("x-forwarded-for") {
                format!("{:?}", forwards)
            } else {
                address.to_string()
            }
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
    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, Error> {
        *data.response.status_mut() = StatusCode::NOT_FOUND;
        *data.response.body_mut() = Bytes::from(format!(
            "Failed to find Path: {}",
            data.request.request.uri().path()
        ))
        .stream_body();
        Ok(data)
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
            .ok_or(Error::new(ErrorKind::NotFound, "Failed to find State"))?
            .get::<Arc<T>>()
            .cloned()
            .map(State)
            .ok_or(Error::new(ErrorKind::NotFound, "Failed to find State"))
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

#[async_trait]
impl<'a> FromRequest<'a> for &'a IncomingRequest {
    async fn from_request(
        request: &'a mut ServiceRequest,
        _: &'a str,
    ) -> Result<&'a IncomingRequest, Error> {
        Ok(&request.request)
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
impl FromBody for Bytes {
    async fn from_body(body: &mut BodyType) -> Result<Self, Error> {
        body_to_bytes(body).await
    }
}

#[async_trait::async_trait]
impl FromBody for Vec<u8> {
    async fn from_body(body: &mut BodyType) -> Result<Self, Error> {
        body_to_bytes(body).await.map(|b| b.to_vec())
    }
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
        BodyType::Empty => Ok(Bytes::new()),
    }
}
