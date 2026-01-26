use crate::services::body::{BodyType, MutBody};
use crate::{IntoStreamBody, StreamingBody};
use http::{response::Parts, HeaderMap, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Bytes;
use std::mem::replace;

pub enum ResponseType {
    Stream(Response<StreamingBody>),
    Sized(Response<Full<Bytes>>),
    Consumed(Parts),
    Empty(Response<()>),
}

pub struct ServiceResponse {
    response_type: ResponseType,
}

impl Default for ServiceResponse {
    fn default() -> Self {
        Self::new()
    }
}
impl ServiceResponse {
    pub fn new() -> Self {
        Self {
            response_type: ResponseType::Empty(Response::new(())),
        }
    }
    pub fn set_response(&mut self, outgoing: ResponseType) {
        self.response_type = outgoing;
    }
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        match &mut self.response_type {
            ResponseType::Stream(r) => r.headers_mut(),
            ResponseType::Sized(r) => r.headers_mut(),
            ResponseType::Consumed(r) => &mut r.headers,
            ResponseType::Empty(r) => r.headers_mut(),
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

    pub fn status(&self) -> StatusCode {
        match &self.response_type {
            ResponseType::Stream(r) => r.status(),
            ResponseType::Sized(r) => r.status(),
            ResponseType::Consumed(r) => r.status,
            ResponseType::Empty(r) => r.status(),
        }
    }

    pub fn set_body(&mut self, body: BodyType) {
        fn handle_body(parts: Parts, body_type: BodyType) -> ResponseType {
            match body_type {
                BodyType::Stream(b) => ResponseType::Stream(Response::from_parts(parts, b)),
                BodyType::Sized(b) => ResponseType::Sized(Response::from_parts(parts, b)),
                BodyType::Empty => ResponseType::Empty(Response::from_parts(parts, ())),
            }
        }
        match replace(
            &mut self.response_type,
            ResponseType::Empty(Response::new(())),
        ) {
            ResponseType::Stream(r) => {
                let (parts, _) = r.into_parts();
                let _ = replace(&mut self.response_type, handle_body(parts, body));
            }
            ResponseType::Sized(r) => {
                let (parts, _) = r.into_parts();
                let _ = replace(&mut self.response_type, handle_body(parts, body));
            }
            ResponseType::Consumed(parts) => {
                let _ = replace(&mut self.response_type, handle_body(parts, body));
            }
            ResponseType::Empty(r) => {
                let (parts, _) = r.into_parts();
                let _ = replace(&mut self.response_type, handle_body(parts, body));
            }
        }
    }
    pub fn consume(&mut self) -> BodyType {
        let (response_type, body) = match replace(
            &mut self.response_type,
            ResponseType::Empty(Response::default()),
        ) {
            ResponseType::Sized(r) => {
                let (parts, body) = r.into_parts();
                let body = BodyType::Sized(body);
                (ResponseType::Consumed(parts), body)
            }
            ResponseType::Stream(r) => {
                let (parts, body) = r.into_parts();
                (ResponseType::Consumed(parts), BodyType::Stream(body))
            }
            ResponseType::Consumed(parts) => (ResponseType::Consumed(parts), BodyType::Empty),
            ResponseType::Empty(r) => (ResponseType::Empty(r), BodyType::Empty),
        };
        self.response_type = response_type;
        body
    }
}
impl MutBody for ServiceResponse {
    fn consume(&mut self) -> BodyType {
        match replace(
            &mut self.response_type,
            ResponseType::Empty(Response::new(())),
        ) {
            ResponseType::Sized(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.response_type, ResponseType::Consumed(parts));
                BodyType::Sized(body)
            }
            ResponseType::Stream(r) => {
                let (parts, body) = r.into_parts();
                let _ = replace(&mut self.response_type, ResponseType::Consumed(parts));
                BodyType::Stream(body)
            }
            ResponseType::Consumed(parts) => {
                let _ = replace(&mut self.response_type, ResponseType::Consumed(parts));
                BodyType::Empty
            }
            ResponseType::Empty(r) => {
                let (parts, _) = r.into_parts();
                let _ = replace(&mut self.response_type, ResponseType::Consumed(parts));
                BodyType::Empty
            }
        }
    }
    fn set_body(&mut self, body: BodyType) {
        let (parts, _) = match replace(
            &mut self.response_type,
            ResponseType::Empty(Response::new(())),
        ) {
            ResponseType::Sized(r) => {
                let (parts, body) = r.into_parts();
                (parts, BodyType::Sized(body))
            }
            ResponseType::Stream(r) => {
                let (parts, body) = r.into_parts();
                (parts, BodyType::Stream(body))
            }
            ResponseType::Consumed(parts) => (parts, BodyType::Empty),
            ResponseType::Empty(r) => (r.into_parts().0, BodyType::Empty),
        };
        match body {
            BodyType::Sized(s) => {
                let _ = replace(
                    &mut self.response_type,
                    ResponseType::Sized(Response::from_parts(parts, s)),
                );
            }
            BodyType::Stream(s) => {
                let _ = replace(
                    &mut self.response_type,
                    ResponseType::Stream(Response::from_parts(parts, s)),
                );
            }
            BodyType::Empty => {
                let _ = replace(
                    &mut self.response_type,
                    ResponseType::Sized(Response::from_parts(parts, Full::new(Bytes::new()))),
                );
            }
        }
    }
}
impl From<ServiceResponse> for Response<StreamingBody> {
    fn from(value: ServiceResponse) -> Self {
        match value.response_type {
            ResponseType::Stream(r) => {
                let (parts, body) = r.into_parts();
                Response::from_parts(parts, body)
            }
            ResponseType::Sized(r) => {
                let (parts, body) = r.into_parts();
                Response::from_parts(parts, body.stream_body())
            }
            ResponseType::Consumed(parts) => {
                Response::from_parts(parts, Full::new(Bytes::new()).stream_body())
            }
            ResponseType::Empty(r) => {
                let (parts, _) = r.into_parts();
                Response::from_parts(parts, Full::new(Bytes::new()).stream_body())
            }
        }
    }
}
