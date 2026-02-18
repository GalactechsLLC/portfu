use crate::service::StreamingBody;
use crate::stream::IntoStreamBody;
use http::HeaderValue;
use http::header::Entry;
use http::response::Parts;
use http_body::Body;
use http_body_util::Full;
use hyper::body::Bytes;
use log::error;

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
        Self { response_type }
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
impl<T: AsRef<[u8]>> From<T> for Response {
    fn from(value: T) -> Self {
        Self::ok(value)
    }
}
