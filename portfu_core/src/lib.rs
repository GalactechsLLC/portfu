pub mod cache;
pub mod editable;
pub mod files;
pub mod filters;
pub mod npm_service;
pub mod routes;
pub mod server;
pub mod service;
pub mod signal;
pub mod sockets;
mod ssl;
pub mod task;
pub mod wrappers;

use crate::editable::EditResult;
use crate::server::Server;
use crate::service::{
    BodyType, IncomingRequest, RefBodyType, Service, ServiceRequest, ServiceResponse,
};
use crate::task::Task;
use async_trait::async_trait;
use futures_util::{Stream, TryStreamExt};
use http::Extensions;
use http_body::Frame;
use http_body_util::Full;
use http_body_util::{BodyExt, BodyStream, StreamBody};
use hyper::body::{Bytes, Incoming};
use log::trace;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::fmt::{Debug, Display, Formatter};
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

pub enum ServiceType {
    File,
    Folder,
    API,
}

#[async_trait]
pub trait ServiceHandler {
    fn name(&self) -> &str;
    async fn handle(&self, data: ServiceData) -> Result<ServiceData, (ServiceData, Error)>;
    fn is_editable(&self) -> bool {
        false
    }
    fn service_type(&self) -> ServiceType;
    async fn current_value(&self) -> EditResult {
        EditResult::NotEditable
    }
    async fn update_value(&self, new_value: Vec<u8>, current_value: Option<Vec<u8>>) -> EditResult {
        trace!(
            "Bytes sent to not Editable Service: {:?} - Current Value {:?}",
            new_value,
            current_value
        );
        EditResult::NotEditable
    }
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

    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, (ServiceData, Error)> {
        data.response
            .set_body(BodyType::Sized(Full::new(Bytes::from_static(
                self.1.as_bytes(),
            ))));
        Ok(data)
    }

    fn service_type(&self) -> ServiceType {
        ServiceType::File
    }
}

#[async_trait]
impl ServiceHandler for (String, String) {
    fn name(&self) -> &str {
        &self.0
    }

    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, (ServiceData, Error)> {
        data.response
            .set_body(BodyType::Sized(Full::new(Bytes::from(self.1.clone()))));
        Ok(data)
    }

    fn service_type(&self) -> ServiceType {
        ServiceType::File
    }
}

#[async_trait]
impl ServiceHandler for (&'static str, &'static [u8]) {
    fn name(&self) -> &str {
        self.0
    }

    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, (ServiceData, Error)> {
        data.response
            .set_body(BodyType::Sized(Full::new(Bytes::from_static(self.1))));
        Ok(data)
    }

    fn service_type(&self) -> ServiceType {
        ServiceType::File
    }
}
pub type BoxedBody =
    Box<dyn hyper::body::Body<Data = Bytes, Error = IntoStreamError> + Send + Sync + 'static>;
pub type PinnedBody = Pin<BoxedBody>;
pub type StreamingBody = StreamBody<BodyStream<PinnedBody>>;

type IntoStreamError = &'static str;

pub trait IntoStreamBody {
    type Data;
    type Error;
    fn stream_body(self) -> StreamingBody;
}

impl IntoStreamBody for Bytes {
    type Data = Bytes;
    type Error = IntoStreamError;
    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            Full::new(self).map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl IntoStreamBody for String {
    type Data = Bytes;
    type Error = IntoStreamError;
    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            Full::new(Bytes::from(self)).map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl<'a> IntoStreamBody for &'a str {
    type Data = Bytes;
    type Error = IntoStreamError;
    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            Full::new(Bytes::from(self.to_string()))
                .map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl IntoStreamBody for Vec<u8> {
    type Data = Bytes;
    type Error = IntoStreamError;
    fn stream_body(self) -> StreamingBody {
        Bytes::from(self).stream_body()
    }
}

impl IntoStreamBody for Full<Bytes> {
    type Data = Bytes;
    type Error = IntoStreamError;

    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            self.map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl IntoStreamBody for Incoming {
    type Data = Bytes;
    type Error = IntoStreamError;

    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            self.map_err(|_| "Failed to Convert Incoming into ServiceBody"),
        )))
    }
}

pub fn bytes_stream_to_body<
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + Sync + 'static,
>(
    bytes_stream: S,
) -> StreamingBody {
    let incoming = bytes_stream
        .map_ok(Frame::data)
        .map_err(|_| "Failed to Read Byte Stream");
    let stream_body = StreamBody::new(incoming);
    StreamBody::new(BodyStream::new(Box::pin(stream_body)))
}

pub struct ServiceData {
    pub server: Arc<Server>,
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
                address.ip().to_string()
            }
        } else {
            address.ip().to_string()
        }
    }
}

pub trait ServiceRegister {
    fn register(self, service_registry: &mut ServiceRegistry, shared_state: Extensions);
}

pub static mut STATIC_REGISTRY: Lazy<ServiceRegistry> = Lazy::new(|| ServiceRegistry {
    services: vec![],
    tasks: vec![],
    default_service: None,
});

#[derive(Clone, Debug, Default)]
pub struct ServiceRegistry {
    pub services: Vec<Arc<Service>>,
    pub default_service: Option<Arc<Service>>,
    pub tasks: Vec<Arc<Task>>,
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
impl Display for Path {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
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
    async fn from_body(body: &mut RefBodyType) -> Result<Self, Error>
    where
        Self: Sized;
}

#[async_trait::async_trait]
impl FromBody for Bytes {
    async fn from_body(body: &mut RefBodyType) -> Result<Self, Error> {
        body_to_bytes(body).await
    }
}

#[async_trait::async_trait]
impl FromBody for Vec<u8> {
    async fn from_body(body: &mut RefBodyType) -> Result<Self, Error> {
        body_to_bytes(body).await.map(|b| b.to_vec())
    }
}

#[async_trait::async_trait]
impl FromBody for String {
    async fn from_body(body: &mut RefBodyType) -> Result<Self, Error> {
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
impl<T> FromBody for Json<Option<T>>
where
    T: for<'a> Deserialize<'a>,
{
    async fn from_body(body: &mut RefBodyType) -> Result<Self, Error> {
        let bytes = body_to_bytes(body).await?;
        if bytes.is_empty() || bytes.eq_ignore_ascii_case("{}".as_bytes()) {
            return Ok(Json(None));
        }
        serde_json::from_slice(bytes.as_ref())
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("Failed to parse body as JSON: {e:?}"),
                )
            })
            .map(Some)
            .map(Json)
    }
}
#[async_trait::async_trait]
impl<'r, T: for<'a> Deserialize<'a>> FromRequest<'r> for Json<Option<T>> {
    async fn from_request(request: &'r mut ServiceRequest, _: &'r str) -> Result<Self, Error> {
        let bytes = body_to_bytes(&mut request.request.body()).await?;
        if bytes.is_empty() || bytes.eq_ignore_ascii_case("{}".as_bytes()) {
            return Ok(Json(None));
        }
        serde_json::from_slice(bytes.as_ref())
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("Failed to parse body as JSON: {e:?}"),
                )
            })
            .map(Some)
            .map(Json)
    }
}

pub struct Query<T: for<'a> Deserialize<'a>>(T);
impl<T: for<'a> Deserialize<'a>> Query<T> {
    pub fn inner(self) -> T {
        self.0
    }
}
#[async_trait::async_trait]
impl<'r, T: for<'a> Deserialize<'a>> FromRequest<'r> for Query<Option<T>> {
    async fn from_request(request: &'r mut ServiceRequest, _: &'r str) -> Result<Self, Error> {
        if let Some(query) = request.request.uri().query() {
            serde_urlencoded::from_str(query)
                .map_err(|e| {
                    Error::new(
                        ErrorKind::InvalidInput,
                        format!("Failed to parse query string: {e:?}"),
                    )
                })
                .map(Some)
                .map(Query)
        } else {
            Ok(Query(None))
        }
    }
}

macro_rules! from_body {
    ($int:ident) => {
        #[async_trait::async_trait]
        impl FromBody for $int {
            async fn from_body(body: &mut RefBodyType) -> Result<Self, Error> {
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

async fn body_to_bytes(body: &mut RefBodyType<'_>) -> Result<Bytes, Error> {
    match body {
        RefBodyType::Sized(b) => b.collect().await.map(|v| v.to_bytes()).map_err(|e| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("Failed to read body: {e:?}"),
            )
        }),
        RefBodyType::Stream(b) => b.collect().await.map(|v| v.to_bytes()).map_err(|e| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("Failed to read body: {e:?}"),
            )
        }),
        RefBodyType::Empty => Ok(Bytes::new()),
    }
}
