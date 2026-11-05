use crate::service::StreamingBody;
use crate::stream::IntoStreamBody;
use http::header::{CONTENT_TYPE, Entry};
use http::response::Parts;
use http::{HeaderMap, HeaderValue, StatusCode};
use http_body::Body;
use http_body_util::Full;
use hyper::body::{Bytes, SizeHint};
use log::error;
use serde::Serialize;

const TEXT_PLAIN_UTF8: &str = "text/plain; charset=utf-8";
const APPLICATION_JSON: &str = "application/json";
const APPLICATION_OCTET_STREAM: &str = "application/octet-stream";

pub trait Serialized: Serialize {}

pub trait IntoResponse {
    fn into_response(self) -> Response;
}

pub trait ResponseError: std::error::Error {
    fn status_code(&self) -> StatusCode;

    fn error_response(&self) -> Response {
        Response::from_status_and_message(self.status_code(), self.to_string())
    }
}

impl<T: ResponseError> IntoResponse for T {
    fn into_response(self) -> Response {
        self.error_response()
    }
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

pub enum ResponseType {
    Stream(http::Response<StreamingBody>),
    Sized(http::Response<Full<Bytes>>),
    Consumed(Parts),
    Empty(http::Response<()>),
}

pub struct Response {
    response_type: ResponseType,
}
impl Default for Response {
    fn default() -> Self {
        Self {
            response_type: ResponseType::Empty(http::Response::new(())),
        }
    }
}
impl Response {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn headers(&self) -> &HeaderMap<HeaderValue> {
        match &self.response_type {
            ResponseType::Stream(r) => r.headers(),
            ResponseType::Sized(r) => r.headers(),
            ResponseType::Consumed(r) => &r.headers,
            ResponseType::Empty(r) => r.headers(),
        }
    }
    pub fn headers_mut(&mut self) -> &mut HeaderMap<HeaderValue> {
        match &mut self.response_type {
            ResponseType::Stream(r) => r.headers_mut(),
            ResponseType::Sized(r) => r.headers_mut(),
            ResponseType::Consumed(r) => &mut r.headers,
            ResponseType::Empty(r) => r.headers_mut(),
        }
    }
    pub fn status(&self) -> StatusCode {
        match &self.response_type {
            ResponseType::Stream(r) => r.status(),
            ResponseType::Sized(r) => r.status(),
            ResponseType::Consumed(r) => r.status,
            ResponseType::Empty(r) => r.status(),
        }
    }
    pub fn status_mut(&mut self) -> &mut StatusCode {
        match &mut self.response_type {
            ResponseType::Stream(r) => r.status_mut(),
            ResponseType::Sized(r) => r.status_mut(),
            ResponseType::Consumed(r) => &mut r.status,
            ResponseType::Empty(r) => r.status_mut(),
        }
    }
    pub fn body_size_hint(&self) -> SizeHint {
        match &self.response_type {
            ResponseType::Stream(r) => r.body().size_hint(),
            ResponseType::Sized(r) => r.body().size_hint(),
            ResponseType::Consumed(_) | ResponseType::Empty(_) => SizeHint::with_exact(0),
        }
    }
    pub fn content_type(mut self, value: &'static str) -> Self {
        if let Ok(value) = HeaderValue::from_str(value) {
            self.headers_mut().insert(CONTENT_TYPE, value);
        }
        self
    }
    pub fn json<T: Serialize>(value: T) -> Self {
        Json::from(value).into()
    }
    pub fn from_status_and_message<T: AsRef<[u8]>>(status: http::StatusCode, msg: T) -> Self {
        let bytes = msg.as_ref();
        let response_type = if bytes.is_empty() {
            ResponseType::Empty(http::Response::new(()))
        } else {
            match http::Response::builder()
                .status(status)
                .body(bytes.to_vec().into())
            {
                Ok(r) => ResponseType::Sized(r),
                Err(e) => {
                    error!("Failed to build known 404 response: {e}");
                    ResponseType::Empty(http::Response::new(()))
                }
            }
        };
        let mut response = Self { response_type };
        *response.status_mut() = status;
        if !bytes.is_empty() && !response.headers().contains_key(CONTENT_TYPE) {
            response
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(TEXT_PLAIN_UTF8));
        }
        response
    }
    pub fn not_found<T: AsRef<[u8]>>(msg: T) -> Self {
        Self::from_status_and_message(http::StatusCode::NOT_FOUND, msg)
    }
    pub fn internal_error<T: AsRef<[u8]>>(msg: T) -> Self {
        Self::from_status_and_message(http::StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
    pub fn ok<T: AsRef<[u8]>>(msg: T) -> Self {
        Self::from_status_and_message(http::StatusCode::OK, msg)
    }
}
impl From<Response> for http::Response<StreamingBody> {
    fn from(value: Response) -> Self {
        match value.response_type {
            ResponseType::Stream(r) => {
                let (parts, body) = r.into_parts();
                http::Response::from_parts(parts, body)
            }
            ResponseType::Sized(r) => {
                let (mut parts, body) = r.into_parts();
                let size = if let Some(exact) = body.size_hint().exact() {
                    exact
                } else if let Some(upper) = body.size_hint().upper() {
                    upper
                } else if body.size_hint().lower() > 0 {
                    body.size_hint().lower()
                } else {
                    0
                };
                if size > 0
                    && let Entry::Vacant(h) = parts.headers.entry(http::header::CONTENT_LENGTH)
                {
                    h.insert(HeaderValue::from(size));
                }
                http::Response::from_parts(parts, body.stream_body())
            }
            ResponseType::Consumed(mut parts) => {
                parts
                    .headers
                    .insert(http::header::CONTENT_LENGTH, HeaderValue::from(0));
                http::Response::from_parts(parts, Full::new(Bytes::new()).stream_body())
            }
            ResponseType::Empty(r) => {
                let (mut parts, _) = r.into_parts();
                parts
                    .headers
                    .insert(http::header::CONTENT_LENGTH, HeaderValue::from(0));
                http::Response::from_parts(parts, Full::new(Bytes::new()).stream_body())
            }
        }
    }
}
impl From<http::Response<Full<Bytes>>> for Response {
    fn from(value: http::Response<Full<Bytes>>) -> Self {
        Self {
            response_type: ResponseType::Sized(value),
        }
    }
}
impl From<http::Response<StreamingBody>> for Response {
    fn from(value: http::Response<StreamingBody>) -> Self {
        Self {
            response_type: ResponseType::Stream(value),
        }
    }
}
impl From<http::Response<()>> for Response {
    fn from(value: http::Response<()>) -> Self {
        Self {
            response_type: ResponseType::Empty(value),
        }
    }
}
impl From<()> for Response {
    fn from(_: ()) -> Self {
        Self::default()
    }
}

impl From<String> for Response {
    fn from(value: String) -> Self {
        Self::ok(value).content_type(TEXT_PLAIN_UTF8)
    }
}

impl From<&str> for Response {
    fn from(value: &str) -> Self {
        Self::ok(value).content_type(TEXT_PLAIN_UTF8)
    }
}

impl From<Vec<u8>> for Response {
    fn from(value: Vec<u8>) -> Self {
        Self::ok(value).content_type(APPLICATION_OCTET_STREAM)
    }
}

impl From<&[u8]> for Response {
    fn from(value: &[u8]) -> Self {
        Self::ok(value).content_type(APPLICATION_OCTET_STREAM)
    }
}

impl From<Bytes> for Response {
    fn from(value: Bytes) -> Self {
        Self::ok(value).content_type(APPLICATION_OCTET_STREAM)
    }
}

pub struct Json<T: Serialize>(pub T);
impl<T: Serialize> From<T> for Json<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}
impl<T: Serialize> From<Json<T>> for Response {
    fn from(value: Json<T>) -> Self {
        match serde_json::to_string(&value.0) {
            Ok(json) => Self::ok(json).content_type(APPLICATION_JSON),
            Err(e) => {
                error!("Failed to serialize JSON: {e}");
                Self::internal_error("Failed to serialize JSON")
            }
        }
    }
}

impl From<serde_json::Value> for Response {
    fn from(value: serde_json::Value) -> Self {
        Self::json(value)
    }
}

impl<T: Serialized> From<T> for Response {
    fn from(value: T) -> Self {
        Self::json(value)
    }
}

pub type JsonResponse<T> = Json<T>;
