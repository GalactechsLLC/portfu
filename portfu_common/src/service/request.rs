use crate::router::route::Route;
use crate::service::{DEFAULT_URI, StreamingBody};
use http::request::Parts;
use http::{Extensions, HeaderMap, HeaderValue, Method, Uri};
use http_body_util::Full;
use hyper::body::Bytes;
use std::pin::Pin;
use std::sync::Arc;

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

    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        match &self.request_type {
            RequestType::Sized(r) => r.extensions().get::<T>(),
            RequestType::Stream(r) => r.extensions().get::<T>(),
            RequestType::Consumed(r) => r.extensions.get::<T>(),
            RequestType::Empty(_) => None,
        }
    }
}
