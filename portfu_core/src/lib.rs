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
use crate::filters::FilterFn;
use crate::server::Server;
use crate::service::{
    BodyType, IncomingRequest, RefBodyType, Service, ServiceRequest, ServiceResponse,
};
use crate::task::Task;
use crate::wrappers::WrapperFn;
use async_trait::async_trait;
use futures_util::{Stream, TryStreamExt};
use http::Extensions;
use http_body::Frame;
use http_body_util::Full;
use http_body_util::{BodyExt, BodyStream, StreamBody};
use hyper::body::{Bytes, Incoming};
use log::{debug, trace};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::fmt::{Debug, Display, Formatter};
use std::io::{Error, ErrorKind};
use std::net::{IpAddr, SocketAddr};
use std::ops::Deref;
use std::pin::Pin;
use std::str::FromStr;
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

impl IntoStreamBody for &str {
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
        let remote = if let Some(real_ip) = self.request.request.headers().get("x-real-ip") {
            format!("{real_ip:?}")
        } else {
            address.ip().to_string()
        };
        debug!("Found Remote IP: {remote}");
        if is_cloudflare(&remote) {
            debug!("Detected Cloudflare");
            if let Some(real_ip) = self.request.request.headers().get("cf-connecting-ip") {
                let ip = format!("{real_ip:?}");
                debug!("Cloudflare: Real IP: {ip}");
                ip
            } else {
                address.ip().to_string()
            }
        } else {
            remote
        }
    }
}

pub fn is_cloudflare(remote_address: &str) -> bool {
    let ip = match IpAddr::from_str(remote_address) {
        Ok(ip) => ip,
        Err(_) => return false,
    };

    const CLOUDFLARE_CIDRS: &[&str] = &[
        // IPv4
        "173.245.48.0/20",
        "103.21.244.0/22",
        "103.22.200.0/22",
        "103.31.4.0/22",
        "141.101.64.0/18",
        "108.162.192.0/18",
        "190.93.240.0/20",
        "188.114.96.0/20",
        "197.234.240.0/22",
        "198.41.128.0/17",
        "162.158.0.0/15",
        "104.16.0.0/13",
        "104.24.0.0/14",
        "172.64.0.0/13",
        "131.0.72.0/22",
        // IPv6
        "2400:cb00::/32",
        "2606:4700::/32",
        "2803:f800::/32",
        "2405:b500::/32",
        "2405:8100::/32",
        "2a06:98c0::/29",
        "2c0f:f248::/32",
    ];

    CLOUDFLARE_CIDRS.iter().any(|cidr| match parse_cidr(cidr) {
        Some((base, prefix)) => ip_in_prefix(&ip, &base, prefix),
        None => false,
    })
}

fn parse_cidr(cidr: &str) -> Option<(IpAddr, u8)> {
    let (addr_str, prefix_str) = cidr.split_once('/')?;
    let addr = IpAddr::from_str(addr_str).ok()?;
    let prefix_len = prefix_str.parse().ok()?;
    Some((addr, prefix_len))
}

fn ip_in_prefix(ip: &IpAddr, base: &IpAddr, prefix_len: u8) -> bool {
    match (ip, base) {
        (IpAddr::V4(ip), IpAddr::V4(base)) => {
            let ip = u32::from_be_bytes(ip.octets());
            let base = u32::from_be_bytes(base.octets());
            let mask = if prefix_len == 0 {
                0
            } else {
                u32::MAX << (32 - prefix_len)
            };
            (ip & mask) == (base & mask)
        }
        (IpAddr::V6(ip), IpAddr::V6(base)) => {
            let ip = u128::from_be_bytes(ip.octets());
            let base = u128::from_be_bytes(base.octets());
            let mask = if prefix_len == 0 {
                0
            } else {
                u128::MAX << (128 - prefix_len)
            };
            (ip & mask) == (base & mask)
        }
        _ => false, // mismatch between IPv4 and IPv6
    }
}

pub trait ServiceRegister {
    fn register(self, service_registry: &mut ServiceRegistry, shared_state: Extensions);
}

pub static mut STATIC_REGISTRY: Lazy<ServiceRegistry> = Lazy::new(|| ServiceRegistry {
    services: vec![],
    tasks: vec![],
    wrappers: vec![],
    filters: vec![],
    default_service: None,
});

#[derive(Clone, Debug, Default)]
pub struct ServiceRegistry {
    pub services: Vec<Arc<Service>>,
    pub default_service: Option<Arc<Service>>,
    pub tasks: Vec<Arc<Task>>,
    pub filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    pub wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
}
impl ServiceRegistry {
    pub fn register(&mut self, mut service: Service) {
        service.wrappers.extend(self.wrappers.clone());
        service.filters.extend(self.filters.clone());
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

impl<T: Send + Sync + 'static> Deref for State<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref()
    }
}
#[async_trait]
impl<'a, T: Send + Sync + 'static> FromRequest<'a> for State<T> {
    async fn from_request(request: &'a mut ServiceRequest, _: &'a str) -> Result<Self, Error> {
        request
            .request
            .extensions()
            .ok_or(Error::new(
                ErrorKind::NotFound,
                format!(
                    "Failed to find State of type {}",
                    std::any::type_name::<T>()
                ),
            ))?
            .get::<Arc<T>>()
            .cloned()
            .map(State)
            .ok_or(Error::new(
                ErrorKind::NotFound,
                format!(
                    "Failed to find State of type {}",
                    std::any::type_name::<T>()
                ),
            ))
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
            serde_html_form::from_str(query)
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
