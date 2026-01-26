use crate::router::filters::{FilterFn, FilterResult};
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::router::routes::Route;
use crate::{ServiceData, ServiceHandler, ServiceRegister, ServiceRegistry};
use http::{Extensions, HeaderMap, HeaderValue, Request};
use hyper::body::Incoming;
use std::io::Error;
use std::sync::Arc;
use uuid::Uuid;

pub mod body;
pub mod builder;
pub mod group;
pub mod request;
pub mod response;

pub type RequestHeaders = HeaderMap<HeaderValue>;
pub type ResponseHeaders = HeaderMap<HeaderValue>;
#[derive(Debug, Clone)]
pub struct Service {
    pub path: Arc<Route>,
    pub name: String,
    pub uuid: Uuid,
    pub shared_state: Extensions,
    pub filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    pub wrappers: Vec<Arc<dyn Middleware + Sync + Send>>,
    pub handler: Option<Arc<dyn ServiceHandler + Send + Sync>>,
}
impl Service {
    pub async fn handles(&self, req: &Request<Incoming>) -> bool {
        if self.path.matches(req.uri().path()) {
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
    pub async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, (ServiceData, Error)> {
        for func in self.wrappers.iter() {
            match func.before(&mut data).await {
                Ok(MiddlewareResult::Continue) => {}
                Ok(MiddlewareResult::Return) => {
                    return Ok(data);
                }
                Err(e) => return Err((data, e)),
            }
        }
        if let Some(handler) = self.handler.as_ref() {
            data = handler.handle(data).await?;
        }
        for func in self.wrappers.iter() {
            match func.after(&mut data).await {
                Ok(MiddlewareResult::Continue) => {}
                Ok(MiddlewareResult::Return) => {
                    return Ok(data);
                }
                Err(e) => return Err((data, e)),
            };
        }
        Ok(data)
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub fn uuid(&self) -> &Uuid {
        &self.uuid
    }
}
impl ServiceRegister for Service {
    fn register(mut self, service_registry: &mut ServiceRegistry, shared_state: Extensions) {
        self.shared_state.extend(shared_state);
        service_registry.register(self)
    }
}
