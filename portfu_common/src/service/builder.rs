use crate::router::filter::traits::Filter;
use crate::router::middleware::Middleware;
use crate::router::route::Route;
use crate::service::Service as ServiceImpl;
use crate::service::traits::Service;
use std::sync::Arc;
use uuid::Uuid;

pub struct ServiceBuilder {
    route: Route,
    name: Option<String>,
    scope: String,
    domains: Vec<String>,
    filters: Vec<Arc<dyn Filter + Sync + Send>>,
    middleware: Vec<Arc<dyn Middleware + Sync + Send>>,
    service: Option<Arc<dyn Service + Send + Sync>>,
}
impl ServiceBuilder {
    pub fn new(path: &str) -> Self {
        Self {
            route: Route::new(path.to_string()),
            name: None,
            scope: "default".to_string(),
            domains: vec![],
            filters: vec![],
            middleware: vec![],
            service: None,
        }
    }
    pub fn name<S: AsRef<str>>(mut self, path: S) -> Self {
        self.name = Some(path.as_ref().to_string());
        self
    }
    pub fn scope<S: AsRef<str>>(mut self, scope: S) -> Self {
        self.scope = scope.as_ref().to_string();
        self
    }
    pub fn domain<S: AsRef<str>>(mut self, domain: S) -> Self {
        self.domains.push(domain.as_ref().to_ascii_lowercase());
        self
    }
    pub fn filter(mut self, filter: Arc<dyn Filter + Sync + Send>) -> Self {
        self.filters.push(filter);
        self
    }
    pub fn wrap(mut self, middleware: Arc<dyn Middleware + Sync + Send>) -> Self {
        self.middleware.push(middleware);
        self
    }
    pub fn handler(mut self, service_handler: Arc<dyn Service + Send + Sync>) -> Self {
        self.service = Some(service_handler);
        self
    }
    pub fn build(self) -> ServiceImpl {
        ServiceImpl {
            route: Arc::new(self.route),
            name: self.name.unwrap_or_default(),
            scope: self.scope,
            domains: self.domains,
            uuid: Uuid::new_v4(),
            service: self.service,
            middleware: self.middleware,
            filters: self.filters,
        }
    }
}
