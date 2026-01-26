use crate::router::routes::Route;
use crate::services::body::{BodyType, MutBody, RefBodyType};
use crate::StreamingBody;
use http::{
    request::Parts, Extensions, HeaderMap, HeaderValue, Method, Request, Response, StatusCode, Uri,
};
use http_body::{Body, SizeHint};
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Bytes;
use hyper::upgrade::OnUpgrade;
use log::error;
use once_cell::sync::Lazy;
use std::io::{Error, ErrorKind};
use std::mem::replace;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;

static DEFAULT_URI: Lazy<Uri> = Lazy::new(Uri::default);

pub enum RequestType {
    Stream(Request<StreamingBody>),
    Sized(Request<Full<Bytes>>),
    Consumed(Parts),
    Empty(HeaderMap<HeaderValue>),
}

pub struct ServiceRequest {
    request_type: RequestType,
    route: Arc<Route>,
}
impl ServiceRequest {
    pub fn new(request_type: RequestType, route: Arc<Route>) -> Self {
        ServiceRequest {
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
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        if let Some(ext) = self.extensions() {
            ext.get()
        } else {
            None
        }
    }
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        if let Some(ext) = self.extensions_mut() {
            ext.get_mut()
        } else {
            None
        }
    }
    pub fn insert<T: Clone + Send + Sync + 'static>(&mut self, t: T) -> Option<T> {
        if let Some(ext) = self.extensions_mut() {
            ext.insert(t)
        } else {
            None
        }
    }
    pub fn remove<T: Clone + Send + Sync + 'static>(&mut self) -> Option<T> {
        if let Some(ext) = self.extensions_mut() {
            ext.remove()
        } else {
            None
        }
    }
    pub async fn consume(&mut self) -> Result<BodyType, Error> {
        let (request_type, body) =
            match replace(&mut self.request_type, RequestType::Empty(HeaderMap::new())) {
                RequestType::Sized(r) => {
                    let (parts, body) = r.into_parts();
                    let body = BodyType::Sized(Full::new(match body.collect().await {
                        Ok(b) => b.to_bytes(),
                        Err(_) => {
                            self.request_type = RequestType::Consumed(parts);
                            return Err(Error::new(
                                ErrorKind::InvalidData,
                                "Failed to read all Bytes from Request",
                            ));
                        }
                    }));
                    (RequestType::Consumed(parts), body)
                }
                RequestType::Stream(r) => {
                    let (parts, body) = r.into_parts();
                    (RequestType::Consumed(parts), BodyType::Stream(body))
                }
                RequestType::Consumed(parts) => (RequestType::Consumed(parts), BodyType::Empty),
                RequestType::Empty(headers) => (RequestType::Empty(headers), BodyType::Empty),
            };
        self.request_type = request_type;
        Ok(body)
    }

    pub fn uri(&self) -> &Uri {
        match &self.request_type {
            RequestType::Sized(r) => r.uri(),
            RequestType::Stream(r) => r.uri(),
            RequestType::Consumed(r) => &r.uri,
            RequestType::Empty(_) => &DEFAULT_URI,
        }
    }
    pub fn headers(&self) -> &HeaderMap<HeaderValue> {
        match &self.request_type {
            RequestType::Sized(r) => r.headers(),
            RequestType::Stream(r) => r.headers(),
            RequestType::Consumed(r) => &r.headers,
            RequestType::Empty(headers) => headers,
        }
    }
    pub fn headers_mut(&mut self) -> &mut HeaderMap<HeaderValue> {
        match &mut self.request_type {
            RequestType::Sized(r) => r.headers_mut(),
            RequestType::Stream(r) => r.headers_mut(),
            RequestType::Consumed(r) => &mut r.headers,
            RequestType::Empty(headers) => headers,
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
    pub fn size_hint(&self) -> SizeHint {
        match &self.request_type {
            RequestType::Sized(r) => r.size_hint(),
            RequestType::Stream(r) => r.size_hint(),
            RequestType::Consumed(_) => SizeHint::with_exact(0),
            RequestType::Empty(_) => SizeHint::with_exact(0),
        }
    }
    pub fn extensions(&self) -> Option<&Extensions> {
        match &self.request_type {
            RequestType::Sized(r) => Some(r.extensions()),
            RequestType::Stream(r) => Some(r.extensions()),
            RequestType::Consumed(r) => Some(&r.extensions),
            RequestType::Empty(_) => None,
        }
    }
    pub fn extensions_mut(&mut self) -> Option<&mut Extensions> {
        match &mut self.request_type {
            RequestType::Sized(r) => Some(r.extensions_mut()),
            RequestType::Stream(r) => Some(r.extensions_mut()),
            RequestType::Consumed(r) => Some(&mut r.extensions),
            RequestType::Empty(_) => None,
        }
    }
    pub fn body(&mut self) -> RefBodyType<'_> {
        match &mut self.request_type {
            RequestType::Sized(r) => RefBodyType::Sized(r.body_mut()),
            RequestType::Stream(r) => RefBodyType::Stream(r.body_mut()),
            RequestType::Consumed(_) => RefBodyType::Empty,
            RequestType::Empty(_) => RefBodyType::Empty,
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
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(hyper::header::CONNECTION, "upgrade")
            .header(hyper::header::UPGRADE, "websocket")
            .header("Sec-WebSocket-Accept", &derive_accept_key(key.as_bytes()))
            .body(Full::default())
            .map_err(|e| {
                error!("Failed to build WebSocket Response: {e}");
                ProtocolError::HandshakeIncomplete
            })?;
        match &mut self.request_type {
            RequestType::Stream(request) => Ok((response, hyper::upgrade::on(request))),
            RequestType::Sized(request) => Ok((response, hyper::upgrade::on(request))),
            RequestType::Consumed(parts) => Ok((
                response,
                hyper::upgrade::on(Request::<Empty<()>>::from_parts(
                    parts.clone(),
                    Empty::default(),
                )),
            )),
            RequestType::Empty(_) => Err(ProtocolError::InvalidCloseSequence), //maye a different error? Should not ever happen
        }
    }
}

impl MutBody for ServiceRequest {
    fn consume(&mut self) -> BodyType {
        match replace(&mut self.request_type, RequestType::Empty(HeaderMap::new())) {
            RequestType::Sized(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.request_type, RequestType::Consumed(parts));
                BodyType::Sized(body)
            }
            RequestType::Stream(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.request_type, RequestType::Consumed(parts));
                BodyType::Stream(body)
            }
            RequestType::Consumed(parts) => {
                let _ = replace(&mut self.request_type, RequestType::Consumed(parts));
                BodyType::Empty
            }
            RequestType::Empty(_) => BodyType::Empty,
        }
    }
    fn set_body(&mut self, body: BodyType) {
        let (parts, _) = match replace(&mut self.request_type, RequestType::Empty(HeaderMap::new()))
        {
            RequestType::Sized(r) => {
                let (parts, body) = r.into_parts();
                (parts, BodyType::Sized(body))
            }
            RequestType::Stream(r) => {
                let (parts, body) = r.into_parts();
                (parts, BodyType::Stream(body))
            }
            RequestType::Consumed(parts) => (parts, BodyType::Empty),
            RequestType::Empty(headers) => {
                let mut new_parts = Request::new(()).into_parts().0;
                new_parts.headers = headers;
                (new_parts, BodyType::Empty)
            }
        };
        match body {
            BodyType::Sized(s) => {
                let _ = replace(
                    &mut self.request_type,
                    RequestType::Sized(Request::from_parts(parts, s)),
                );
            }
            BodyType::Stream(s) => {
                let _ = replace(
                    &mut self.request_type,
                    RequestType::Stream(Request::from_parts(parts, s)),
                );
            }
            BodyType::Empty => {
                let _ = replace(
                    &mut self.request_type,
                    RequestType::Sized(Request::from_parts(parts, Full::new(Bytes::new()))),
                );
            }
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
