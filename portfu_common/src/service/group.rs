use crate::router::filter::traits::Filter;
use crate::router::middleware::Middleware;
use crate::server::state::SharedState;
use crate::service::Service;
use http::Extensions;
use std::sync::Arc;

/// A collection of services that share filters and middleware.
///
/// Group configuration applies to services added after it, matching the fluent
/// builder behavior from Portfu 1.x.
#[derive(Default)]
pub struct ServiceGroup {
    services: Vec<Service>,
    shared_state: Extensions,
    filters: Vec<Arc<dyn Filter + Send + Sync>>,
    middleware: Vec<Arc<dyn Middleware + Send + Sync>>,
}

impl ServiceGroup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn service<T: Into<Service>>(mut self, service: T) -> Self {
        self.add_service(service.into());
        self
    }

    pub fn services<I, T>(mut self, services: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Service>,
    {
        for service in services {
            self.add_service(service.into());
        }
        self
    }

    /// Adds state for services in this group to the server's default scope.
    ///
    /// A later registration of the same type replaces an earlier value.
    pub fn shared_state<T: Send + Sync + 'static>(
        mut self,
        value: impl Into<SharedState<T>>,
    ) -> Self {
        let state: SharedState<T> = value.into();
        self.shared_state.insert::<Arc<T>>(state.into());
        self
    }

    pub fn sub_group(mut self, group: ServiceGroup) -> Self {
        let (shared_state, services) = group.into_parts();
        self.shared_state.extend(shared_state);
        for service in services {
            self.add_service(service);
        }
        self
    }

    pub fn filter(mut self, filter: Arc<dyn Filter + Send + Sync>) -> Self {
        self.filters.push(filter);
        self
    }

    pub fn wrap(mut self, middleware: Arc<dyn Middleware + Send + Sync>) -> Self {
        self.middleware.push(middleware);
        self
    }

    fn add_service(&mut self, mut service: Service) {
        service.filters.extend(self.filters.iter().cloned());
        service.middleware.extend(self.middleware.iter().cloned());
        self.services.push(service);
    }

    pub(crate) fn into_parts(self) -> (Extensions, Vec<Service>) {
        (self.shared_state, self.services)
    }
}

impl IntoIterator for ServiceGroup {
    type Item = Service;
    type IntoIter = std::vec::IntoIter<Service>;

    fn into_iter(self) -> Self::IntoIter {
        self.services.into_iter()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/service_group.rs"]
mod tests;
