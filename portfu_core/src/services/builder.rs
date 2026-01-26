use crate::router::filters::FilterFn;
use crate::router::middleware::Middleware;
use crate::router::routes::Route;
use crate::runtime::thread::ServerThread;
use crate::services::Service;
use crate::ServiceHandler;
use http::Extensions;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug)]
pub struct ServiceBuilder {
    path: Route,
    name: Option<String>,
    shared_state: Extensions,
    filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    wrappers: Vec<Arc<dyn Middleware + Sync + Send>>,
    tasks: Vec<Arc<dyn ServerThread + Send + Sync>>,
    handler: Option<Arc<dyn ServiceHandler + Send + Sync>>,
}
impl ServiceBuilder {
    pub fn new(path: &str) -> Self {
        Self {
            path: Route::new(path.to_string()),
            name: None,
            filters: vec![],
            wrappers: vec![],
            tasks: vec![],
            shared_state: Default::default(),
            handler: None,
        }
    }
    pub fn name<S: AsRef<str>>(mut self, path: S) -> Self {
        self.name = Some(path.as_ref().to_string());
        self
    }
    pub fn shared_state<T: Send + Sync + 'static>(mut self, shared_state: T) -> Self {
        self.shared_state.insert(Arc::new(shared_state));
        self
    }
    pub fn extend_state(mut self, shared_state: Extensions) -> Self {
        self.shared_state.extend(shared_state);
        self
    }
    pub fn filter(mut self, filter: Arc<dyn FilterFn + Sync + Send>) -> Self {
        self.filters.push(filter);
        self
    }
    pub fn task(mut self, task: Arc<dyn ServerThread + Sync + Send>) -> Self {
        self.tasks.push(task);
        self
    }
    pub fn wrap(mut self, wrappers: Arc<dyn Middleware + Sync + Send>) -> Self {
        self.wrappers.push(wrappers);
        self
    }
    pub fn handler(mut self, service_handler: Arc<dyn ServiceHandler + Send + Sync>) -> Self {
        self.handler = Some(service_handler);
        self
    }
    pub fn build(self) -> Service {
        Service {
            path: Arc::new(self.path),
            name: self.name.unwrap_or_default(),
            uuid: Uuid::new_v4(),
            shared_state: self.shared_state,
            filters: self.filters,
            wrappers: self.wrappers,
            handler: self.handler,
        }
    }
}
