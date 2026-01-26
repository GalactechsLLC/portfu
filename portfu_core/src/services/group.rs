use crate::router::filters::FilterFn;
use crate::router::middleware::Middleware;
use crate::runtime::thread::{ServerThread, ServerThreadImpl};
use crate::services::Service;
use crate::{ServiceRegister, ServiceRegistry};
use http::Extensions;
use std::sync::Arc;

#[derive(Default)]
pub struct ServiceGroup {
    pub services: Vec<Service>,
    pub shared_state: Extensions,
    pub filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    pub wrappers: Vec<Arc<dyn Middleware + Sync + Send>>,
    pub tasks: Vec<Arc<dyn ServerThread + Sync + Send>>,
}
impl ServiceRegister for ServiceGroup {
    fn register(self, service_registry: &mut ServiceRegistry, shared_state: Extensions) {
        for service in self.services {
            service.register(service_registry, shared_state.clone());
        }
        for task in self.tasks {
            service_registry.tasks.push(Arc::new(ServerThreadImpl {
                name: task.name().to_string(),
                handle: task,
            }));
        }
    }
}
impl ServiceGroup {
    pub fn service<T: ServiceRegister + Into<Service>>(mut self, service: T) -> Self {
        let mut service = service.into();
        service.filters.extend(self.filters.clone());
        service.wrappers.extend(self.wrappers.clone());
        service.shared_state.extend(self.shared_state.clone());
        self.services.push(service);
        self
    }
    pub fn shared_state<T: Send + Sync + 'static>(mut self, shared_state: T) -> Self {
        self.shared_state.insert(Arc::new(shared_state));
        self
    }
    pub fn sub_group<T: Into<ServiceGroup>>(mut self, group: T) -> Self {
        let group = group.into();
        self.shared_state.extend(group.shared_state.clone());
        for service in group.services {
            self = self.service(service);
        }
        for task in group.tasks {
            self = self.task(task);
        }
        self
    }
    pub fn filter(mut self, filter: Arc<dyn FilterFn + Sync + Send>) -> Self {
        self.filters.push(filter);
        self
    }
    pub fn wrap(mut self, wrappers: Arc<dyn Middleware + Sync + Send>) -> Self {
        self.wrappers.push(wrappers);
        self
    }
    pub fn task(mut self, task: Arc<dyn ServerThread + Sync + Send>) -> Self {
        self.tasks.push(task);
        self
    }
}
