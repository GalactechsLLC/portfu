use crate::error::PortfuError;
use crate::router::filter::{FilterResult, traits::Filter};
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::router::route::Route;
use crate::service::request::Request;
use crate::service::response::Response;
use http::{HeaderMap, HeaderValue, Uri};
use http_body_util::{BodyStream, StreamBody};
use hyper::body::Bytes;
use once_cell::sync::Lazy;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

pub mod builder;
pub mod request;
pub mod response;
pub mod state;
pub use state::State;

static DEFAULT_URI: Lazy<Uri> = Lazy::new(Uri::default);
pub type BoxedBody =
    Box<dyn hyper::body::Body<Data = Bytes, Error = &'static str> + Send + Sync + 'static>;
pub type PinnedBody = Pin<BoxedBody>;
pub type StreamingBody = StreamBody<BodyStream<PinnedBody>>;

pub mod traits {
    use crate::error::PortfuError;
    use crate::service::request::Request;
    use crate::service::response::Response;
    use std::pin::Pin;

    pub trait Service {
        fn name(&self) -> &str;
        fn serve<'a>(
            &'a self,
            data: &'a mut Request,
        ) -> Pin<Box<dyn Future<Output = Result<Response, PortfuError>> + 'a + Send>>;
    }
}
pub type RequestHeaders = HeaderMap<HeaderValue>;
pub type ResponseHeaders = HeaderMap<HeaderValue>;
#[derive(Clone)]
pub struct Service {
    route: Arc<Route>,
    name: String,
    scope: String,
    domains: Vec<String>,
    uuid: Uuid,
    filters: Vec<Arc<dyn Filter + Sync + Send>>,
    middleware: Vec<Arc<dyn Middleware + Sync + Send>>,
    service: Option<Arc<dyn traits::Service + Send + Sync>>,
}
impl Service {
    pub async fn serves(&self, req: &Request) -> bool {
        if !self.matches_host(req) {
            return false;
        }
        if self.route.matches(req.uri().path()) {
            for f in self.filters.iter() {
                if f.filter(req).await != FilterResult::Allow {
                    return false;
                }
            }
            true
        } else {
            false
        }
    }
    pub async fn serve(&self, req: &mut Request) -> Result<Response, PortfuError> {
        for func in self.middleware.iter() {
            match func.before(req).await? {
                MiddlewareResult::Continue => {}
                MiddlewareResult::Return(resp) => return Ok(resp),
            }
        }
        let mut response = if let Some(service) = self.service.as_ref() {
            service.serve(req).await?
        } else {
            Response::default()
        };
        for func in self.middleware.iter() {
            match func.after_with_request(req, &mut response).await? {
                MiddlewareResult::Continue => {}
                MiddlewareResult::Return(resp) => return Ok(resp),
            };
        }
        Ok(response)
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub fn scope(&self) -> &str {
        self.scope.as_str()
    }
    pub fn domains(&self) -> &[String] {
        self.domains.as_slice()
    }
    pub fn uuid(&self) -> &Uuid {
        &self.uuid
    }
    pub fn route(&self) -> Arc<Route> {
        self.route.clone()
    }

    fn matches_host(&self, req: &Request) -> bool {
        if self.domains.is_empty() {
            return true;
        }
        let Some(host) = req.host() else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        self.domains.iter().any(|d| d == &host)
    }
}
