use crate::error::PortfuError;
use crate::router::route::Route;
use crate::service::{DEFAULT_URI, StreamingBody};
use http::request::Parts;
use http::{Extensions, HeaderMap, HeaderValue, Method, Uri};
use http_body::Body as HttpBody;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::{Bytes, SizeHint};
use serde::de::DeserializeOwned;
use std::mem::replace;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

pub enum RequestType {
    Stream(http::Request<StreamingBody>),
    Sized(http::Request<Full<Bytes>>),
    Consumed(Parts),
    Empty(HeaderMap<HeaderValue>),
}
pub trait FromRequest<T>: Sized {
    type Error;
    fn try_from<'a>(
        value: &'a mut T,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>>;
}
pub struct Request {
    request_type: RequestType,
    route: Arc<Route>,
}
impl Request {
    pub fn new(request_type: RequestType, route: Arc<Route>) -> Self {
        Request {
            request_type,
            route,
        }
    }
    pub fn request_type(&mut self) -> &mut RequestType {
        &mut self.request_type
    }
    pub fn route(&self) -> Arc<Route> {
        self.route.clone()
    }
    pub fn route_mut(&mut self) -> &mut Arc<Route> {
        &mut self.route
    }

    pub fn uri(&self) -> &Uri {
        match &self.request_type {
            RequestType::Sized(r) => r.uri(),
            RequestType::Stream(r) => r.uri(),
            RequestType::Consumed(r) => &r.uri,
            RequestType::Empty(_) => &DEFAULT_URI,
        }
    }
    pub fn shared_state_mut(&mut self) -> Option<&mut Extensions> {
        match &mut self.request_type {
            RequestType::Sized(r) => Some(r.extensions_mut()),
            RequestType::Stream(r) => Some(r.extensions_mut()),
            RequestType::Consumed(r) => Some(&mut r.extensions),
            RequestType::Empty(_) => None,
        }
    }
    pub fn method(&self) -> &Method {
        match &self.request_type {
            RequestType::Sized(r) => r.method(),
            RequestType::Stream(r) => r.method(),
            RequestType::Consumed(r) => &r.method,
            RequestType::Empty(_) => &Method::OPTIONS,
        }
    }
    pub fn host(&self) -> Option<&str> {
        let from_headers = self
            .headers()
            .get(http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(':').next().unwrap_or(v));
        from_headers.or_else(|| self.uri().host())
    }
    pub fn headers(&self) -> &HeaderMap<HeaderValue> {
        match &self.request_type {
            RequestType::Sized(r) => r.headers(),
            RequestType::Stream(r) => r.headers(),
            RequestType::Consumed(r) => &r.headers,
            RequestType::Empty(h) => h,
        }
    }
    pub fn headers_mut(&mut self) -> &mut HeaderMap<HeaderValue> {
        match &mut self.request_type {
            RequestType::Sized(r) => r.headers_mut(),
            RequestType::Stream(r) => r.headers_mut(),
            RequestType::Consumed(r) => &mut r.headers,
            RequestType::Empty(h) => h,
        }
    }
    pub fn body_size_hint(&self) -> SizeHint {
        match &self.request_type {
            RequestType::Stream(r) => r.body().size_hint(),
            RequestType::Sized(r) => r.body().size_hint(),
            RequestType::Consumed(_) | RequestType::Empty(_) => SizeHint::with_exact(0),
        }
    }
    pub fn set_body_bytes(&mut self, bytes: Bytes) {
        match replace(&mut self.request_type, RequestType::Empty(HeaderMap::new())) {
            RequestType::Stream(r) => {
                let (parts, _) = r.into_parts();
                self.request_type =
                    RequestType::Sized(http::Request::from_parts(parts, Full::new(bytes)));
            }
            RequestType::Sized(r) => {
                let (parts, _) = r.into_parts();
                self.request_type =
                    RequestType::Sized(http::Request::from_parts(parts, Full::new(bytes)));
            }
            RequestType::Consumed(parts) => {
                self.request_type =
                    RequestType::Sized(http::Request::from_parts(parts, Full::new(bytes)));
            }
            RequestType::Empty(mut headers) => {
                if bytes.is_empty() {
                    self.request_type = RequestType::Empty(headers);
                } else {
                    let mut request = http::Request::new(Full::new(bytes));
                    request.headers_mut().extend(headers.drain());
                    self.request_type = RequestType::Sized(request);
                }
            }
        }
    }
    pub async fn consume_body_bytes(&mut self) -> Result<Bytes, PortfuError> {
        match replace(&mut self.request_type, RequestType::Empty(HeaderMap::new())) {
            RequestType::Stream(r) => {
                let (parts, body) = r.into_parts();
                let collected = body.collect().await.map_err(|e| {
                    PortfuError::Internal(format!("Failed to read request body: {e}"))
                })?;
                self.request_type = RequestType::Consumed(parts);
                Ok(collected.to_bytes())
            }
            RequestType::Sized(r) => {
                let (parts, body) = r.into_parts();
                let collected = body.collect().await.map_err(|e| {
                    PortfuError::Internal(format!("Failed to read request body: {e}"))
                })?;
                self.request_type = RequestType::Consumed(parts);
                Ok(collected.to_bytes())
            }
            RequestType::Consumed(parts) => {
                self.request_type = RequestType::Consumed(parts);
                Ok(Bytes::new())
            }
            RequestType::Empty(headers) => {
                self.request_type = RequestType::Empty(headers);
                Ok(Bytes::new())
            }
        }
    }
    pub async fn consume_body_bytes_limited(
        &mut self,
        max_bytes: usize,
        read_timeout: Duration,
    ) -> Result<Bytes, PortfuError> {
        match replace(&mut self.request_type, RequestType::Empty(HeaderMap::new())) {
            RequestType::Stream(r) => {
                let (parts, mut body) = r.into_parts();
                let mut bytes = Vec::new();
                loop {
                    let frame = timeout(read_timeout, body.frame()).await.map_err(|_| {
                        PortfuError::Internal(format!(
                            "Timed out while reading request body after {:?}",
                            read_timeout
                        ))
                    })?;
                    let Some(frame) = frame else {
                        break;
                    };
                    let frame = frame.map_err(|e| {
                        PortfuError::Internal(format!("Failed to read request body: {e}"))
                    })?;
                    if let Some(chunk) = frame.data_ref() {
                        if bytes.len().saturating_add(chunk.len()) > max_bytes {
                            self.request_type = RequestType::Sized(http::Request::from_parts(
                                parts,
                                Full::new(Bytes::new()),
                            ));
                            return Err(PortfuError::Internal(format!(
                                "Request body exceeded {max_bytes} bytes"
                            )));
                        }
                        bytes.extend_from_slice(chunk);
                    }
                }
                let bytes = Bytes::from(bytes);
                self.request_type =
                    RequestType::Sized(http::Request::from_parts(parts, Full::new(bytes.clone())));
                Ok(bytes)
            }
            RequestType::Sized(r) => {
                let (parts, body) = r.into_parts();
                let collected = body.collect().await.map_err(|e| {
                    PortfuError::Internal(format!("Failed to read request body: {e}"))
                })?;
                let bytes = collected.to_bytes();
                if bytes.len() > max_bytes {
                    self.request_type = RequestType::Sized(http::Request::from_parts(
                        parts,
                        Full::new(Bytes::new()),
                    ));
                    return Err(PortfuError::Internal(format!(
                        "Request body exceeded {max_bytes} bytes"
                    )));
                }
                self.request_type =
                    RequestType::Sized(http::Request::from_parts(parts, Full::new(bytes.clone())));
                Ok(bytes)
            }
            RequestType::Consumed(parts) => {
                self.request_type = RequestType::Consumed(parts);
                Ok(Bytes::new())
            }
            RequestType::Empty(headers) => {
                self.request_type = RequestType::Empty(headers);
                Ok(Bytes::new())
            }
        }
    }

    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        match &self.request_type {
            RequestType::Sized(r) => r.extensions().get::<T>(),
            RequestType::Stream(r) => r.extensions().get::<T>(),
            RequestType::Consumed(r) => r.extensions.get::<T>(),
            RequestType::Empty(_) => None,
        }
    }
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        match &mut self.request_type {
            RequestType::Sized(r) => r.extensions_mut().get_mut::<T>(),
            RequestType::Stream(r) => r.extensions_mut().get_mut::<T>(),
            RequestType::Consumed(r) => r.extensions.get_mut::<T>(),
            RequestType::Empty(_) => None,
        }
    }
    pub fn insert<T: Clone + Send + Sync + 'static>(&mut self, value: T) -> Option<T> {
        match &mut self.request_type {
            RequestType::Sized(r) => Some(r.extensions_mut().insert(value)).flatten(),
            RequestType::Stream(r) => Some(r.extensions_mut().insert(value)).flatten(),
            RequestType::Consumed(r) => Some(r.extensions.insert(value)).flatten(),
            RequestType::Empty(_) => None,
        }
    }
    pub fn remove<T: Clone + Send + Sync + 'static>(&mut self) -> Option<T> {
        match &mut self.request_type {
            RequestType::Sized(r) => r.extensions_mut().remove::<T>(),
            RequestType::Stream(r) => r.extensions_mut().remove::<T>(),
            RequestType::Consumed(r) => r.extensions.remove::<T>(),
            RequestType::Empty(_) => None,
        }
    }
}

pub struct Body(pub Bytes);
impl Body {
    pub fn into_bytes(self) -> Bytes {
        self.0
    }
}
impl FromRequest<Request> for Body {
    type Error = PortfuError;
    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move { value.consume_body_bytes().await.map(Body) })
    }
}

pub struct Json<T: DeserializeOwned>(pub T);
impl<T: DeserializeOwned> Json<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}
impl<T: DeserializeOwned + Send + Sync + 'static> FromRequest<Request> for Json<T> {
    type Error = PortfuError;
    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            let bytes = value.consume_body_bytes().await?;
            serde_json::from_slice(bytes.as_ref())
                .map(Json)
                .map_err(|e| PortfuError::Parsing(format!("Failed to parse JSON body: {e}")))
        })
    }
}

pub struct Query<T: DeserializeOwned>(pub T);
impl<T: DeserializeOwned> Query<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}
impl<T: DeserializeOwned + Send + Sync + 'static> FromRequest<Request> for Query<T> {
    type Error = PortfuError;
    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            let query = value.uri().query().unwrap_or_default();
            serde_urlencoded::from_str::<T>(query)
                .map(Query)
                .map_err(|e| PortfuError::Parsing(format!("Failed to parse query string: {e}")))
        })
    }
}
