use crate::router::middleware::Middleware;
use crate::server::Server;
use crate::server::config::{
    ClientAuthConfig, ServerConfig, TlsConfig, TlsIdentity, TlsVersionPolicy,
};
use crate::server::runtime::ServerRuntime;
use crate::server::state::SharedState;
use crate::service::Service;
use crate::service::group::ServiceGroup;
#[cfg(feature = "websocket")]
use crate::websocket::WebSocketAdmissionMiddleware;
use http::Extensions;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::sync::{RwLock, watch};

pub struct ServerBuilder {
    pub run: Arc<AtomicBool>,
    pub config: ServerConfig,
    pub scoped_state: HashMap<String, Extensions>,
    pub services: Vec<Service>,
    pub middleware: Vec<Arc<dyn Middleware + Send + Sync>>,
    pub default_service: Option<Service>,
    pub health_service: Option<Service>,
    shutdown: watch::Sender<bool>,
    #[cfg(feature = "websocket")]
    pub websocket_admission: Vec<Arc<dyn WebSocketAdmissionMiddleware + Send + Sync>>,
}
impl ServerBuilder {
    pub fn new() -> Self {
        let (shutdown, _) = watch::channel(false);
        Self {
            run: Arc::new(AtomicBool::new(true)),
            config: ServerConfig::default(),
            scoped_state: HashMap::new(),
            services: vec![],
            middleware: vec![],
            default_service: None,
            health_service: None,
            shutdown,
            #[cfg(feature = "websocket")]
            websocket_admission: vec![],
        }
    }

    pub fn from_env() -> Self {
        let mut builder = ServerBuilder::new();
        if let Ok(host) = env::var("PORTFU_HOST") {
            builder.config.host = host;
        }
        if let Ok(port) = env::var("PORTFU_PORT").map(|v| v.parse::<u16>())
            && let Ok(port) = port
        {
            builder.config.port = port;
        }
        if let Ok(backlog) = env::var("PORTFU_BACKLOG").map(|v| v.parse::<u32>())
            && let Ok(backlog) = backlog
        {
            builder.config.backlog = backlog;
        }
        if let Ok(acceptors) = env::var("PORTFU_ACCEPTORS").map(|v| v.parse::<usize>())
            && let Ok(acceptors) = acceptors
        {
            builder.config.acceptors = acceptors;
        }
        if let Ok(reuse_port) = env::var("PORTFU_REUSEPORT") {
            builder.config.reuse_port =
                reuse_port == "1" || reuse_port.eq_ignore_ascii_case("true");
        }
        if let Ok(ssl_enabled) = env::var("PORTFU_SSL_ENABLED")
            && (ssl_enabled == "1" || ssl_enabled.eq_ignore_ascii_case("true"))
        {
            builder.config.tls = Some(TlsConfig::default());
        }
        builder
    }
    pub fn host<T: AsRef<str>>(self, host: T) -> Self {
        let mut s = self;
        s.config.host = host.as_ref().to_string();
        s
    }
    pub fn port(self, port: u16) -> Self {
        let mut s = self;
        s.config.port = port;
        s
    }
    pub fn tls(mut self, tls: TlsConfig) -> Self {
        self.config.tls = Some(tls);
        self
    }
    pub fn tls_identity(mut self, identity: TlsIdentity) -> Self {
        self.config
            .tls
            .get_or_insert_with(TlsConfig::default)
            .identities
            .push(identity);
        self
    }
    pub fn client_auth(mut self, client_auth: ClientAuthConfig) -> Self {
        self.config
            .tls
            .get_or_insert_with(TlsConfig::default)
            .client_auth = client_auth;
        self
    }
    pub fn tls_version_policy(mut self, policy: TlsVersionPolicy) -> Self {
        self.config
            .tls
            .get_or_insert_with(TlsConfig::default)
            .versions = policy;
        self
    }
    pub fn tls_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.config
            .tls
            .get_or_insert_with(TlsConfig::default)
            .handshake_timeout = timeout;
        self
    }
    pub fn http_header_read_timeout(mut self, timeout: Duration) -> Self {
        self.config.http_header_read_timeout = timeout;
        self
    }
    pub fn trust_proxy_headers(mut self, trust: bool) -> Self {
        self.config.trust_proxy_headers = trust;
        self
    }
    pub fn shutdown_grace_period(mut self, grace_period: Duration) -> Self {
        self.config.shutdown_grace_period = grace_period;
        self
    }
    pub fn global_state<S: Send + Sync + 'static>(
        mut self,
        value: impl Into<SharedState<S>>,
    ) -> Self {
        self = self.scoped_state("default", value);
        self
    }
    pub fn scoped_state<K: AsRef<str>, S: Send + Sync + 'static>(
        mut self,
        scope: K,
        value: impl Into<SharedState<S>>,
    ) -> Self {
        let state: SharedState<S> = value.into();
        self.scoped_state
            .entry(scope.as_ref().to_string())
            .or_default()
            .insert::<Arc<S>>(state.into());
        self
    }
    pub fn default_service<T: Into<Service>>(mut self, service: T) -> Self {
        self.default_service = Some(service.into());
        self
    }
    pub fn health_service<T: Into<Service>>(mut self, service: T) -> Self {
        self.health_service = Some(service.into());
        self
    }
    pub fn service<T: Into<Service>>(mut self, service: T) -> Self {
        self.services.push(service.into());
        self
    }
    pub fn service_group(mut self, group: ServiceGroup) -> Self {
        let (shared_state, services) = group.into_parts();
        self.scoped_state
            .entry("default".to_string())
            .or_default()
            .extend(shared_state);
        self.services.extend(services);
        self
    }
    pub fn wrap(mut self, middleware: Arc<dyn Middleware + Send + Sync>) -> Self {
        self.middleware.push(middleware);
        self
    }
    #[cfg(feature = "websocket")]
    pub fn websocket_admission(
        mut self,
        middleware: Arc<dyn WebSocketAdmissionMiddleware + Send + Sync>,
    ) -> Self {
        self.websocket_admission.push(middleware);
        self
    }
    pub fn build(self) -> Server {
        Server {
            run: self.run,
            config: self.config,
            scoped_state: Arc::new(RwLock::new(self.scoped_state)),
            services: self.services,
            middleware: self.middleware,
            default_service: self.default_service,
            shutdown: self.shutdown,
            runtime: Arc::new(ServerRuntime::default()),
            #[cfg(feature = "websocket")]
            websocket_admission: self.websocket_admission,
        }
    }
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/server_builder.rs"]
mod tests;
