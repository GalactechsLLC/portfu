use crate::router::filters::Filter;
use crate::router::middleware::Middleware;
use crate::runtime::thread::ServerThreadImpl;
use crate::server::config::{ServerConfig, SslConfig};
use crate::server::state::SharedState;
use crate::server::Server;
use crate::services::Service;
use crate::{ServiceRegister, ServiceRegistry};
use http::Extensions;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type DelayedRegistry = Vec<Box<dyn FnOnce(&mut ServiceRegistry, Extensions)>>;

pub struct ServerBuilder {
    services: ServiceRegistry,
    config: ServerConfig,
    shared_state: Extensions,
    run_handle: Arc<AtomicBool>,
    delayed_registry: DelayedRegistry,
}
impl ServerBuilder {
    pub fn from_config(config: ServerConfig) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }
    pub fn host(self, host: String) -> Self {
        let mut s = self;
        s.config.host = host;
        s
    }
    pub fn port(self, port: u16) -> Self {
        let mut s = self;
        s.config.port = port;
        s
    }
    pub fn ssl_config(self, ssl_config: Option<SslConfig>) -> Self {
        let mut s = self;
        s.config.ssl_config = ssl_config;
        s
    }
    pub fn register<T: ServiceRegister>(self, service: T) -> Self {
        let mut s = self;
        service.register(&mut s.services, s.shared_state.clone());
        s
    }
    pub fn delay_register<T: 'static + ServiceRegister>(self, service: T) -> Self {
        let mut s = self;
        s.delayed_registry.push(Box::new(move |reg, shared| {
            service.register(reg, shared); // consumes the concrete `service`
        }));
        s
    }
    pub fn default_service(self, mut service: Service) -> Self {
        let mut s = self;
        service.shared_state.extend(s.shared_state.clone());
        service.wrappers.extend(s.services.wrappers.clone());
        service.filters.extend(s.services.filters.clone());
        s.services.default_service = Some(Arc::new(service));
        s
    }
    pub fn filter(self, filter: Filter) -> Self {
        let mut s = self;
        s.services.filters.push(Arc::new(filter));
        s
    }
    pub fn wrap(self, wrapper: Arc<dyn Middleware + Sync + Send>) -> Self {
        let mut s = self;
        s.services.wrappers.push(wrapper);
        s
    }
    pub fn task<T: Into<ServerThreadImpl>>(mut self, task: T) -> Self {
        self.services.tasks.push(Arc::new(task.into()));
        self
    }
    pub fn run_handle(mut self, run_handle: Arc<AtomicBool>) -> Self {
        self.run_handle = run_handle;
        self
    }
    pub fn shared_state<T: Send + Sync + 'static>(
        self,
        shared_state: impl Into<SharedState<T>>,
    ) -> Self {
        let mut s = self;
        let state: SharedState<T> = shared_state.into();
        s.shared_state.insert::<Arc<T>>(state.into());
        s
    }
    pub fn build(mut self) -> Server {
        for f in self.delayed_registry {
            f(&mut self.services, self.shared_state.clone());
        }
        Server {
            registry: Arc::new(RwLock::new(self.services)),
            config: self.config,
            run: self.run_handle,
            shared_state: Arc::new(RwLock::new(self.shared_state)),
        }
    }
}
impl Default for ServerBuilder {
    fn default() -> Self {
        Self {
            services: ServiceRegistry {
                services: vec![],
                tasks: vec![],
                filters: vec![],
                wrappers: vec![],
                default_service: None,
            },
            config: ServerConfig::default(),
            shared_state: Extensions::default(),
            run_handle: Arc::new(AtomicBool::new(true)),
            delayed_registry: vec![],
        }
    }
}
