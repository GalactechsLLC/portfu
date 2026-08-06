use crate::router::middleware::Middleware;
use crate::server::Server;
use crate::server::config::{ServerConfig, SslConfig};
use crate::server::state::SharedState;
use crate::service::Service;
use crate::service::group::ServiceGroup;
use http::Extensions;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct ServerBuilder {
    pub run: Arc<AtomicBool>,
    pub config: ServerConfig,
    pub scoped_state: HashMap<String, Extensions>,
    pub services: Vec<Service>,
    pub middleware: Vec<Arc<dyn Middleware + Send + Sync>>,
    pub default_service: Option<Service>,
    pub health_service: Option<Service>,
}
impl ServerBuilder {
    pub fn new() -> Self {
        Self {
            run: Arc::new(AtomicBool::new(true)),
            config: ServerConfig::default(),
            scoped_state: HashMap::new(),
            services: vec![],
            middleware: vec![],
            default_service: None,
            health_service: None,
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
        if let Ok(ssl_enabled) = env::var("PORTFU_SSL_ENABLED") {
            builder.config.enable_ssl =
                ssl_enabled == "1" || ssl_enabled.eq_ignore_ascii_case("true");
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
    pub fn enable_ssl(self, enable_ssl: bool) -> Self {
        let mut s = self;
        s.config.enable_ssl = enable_ssl;
        s
    }
    pub fn ssl_config(self, ssl_config: Option<SslConfig>) -> Self {
        let mut s = self;
        s.config.ssl_config = ssl_config;
        s
    }
    pub fn sni_ssl_config(self, ssl_config: SslConfig) -> Self {
        let mut s = self;
        s.config.sni_ssl_configs.push(ssl_config);
        s
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
    pub fn build(self) -> Server {
        Server {
            run: self.run,
            config: self.config,
            scoped_state: Arc::new(RwLock::new(self.scoped_state)),
            services: self.services,
            middleware: self.middleware,
            default_service: self.default_service,
            health_service: self.health_service,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ServerBuilder;
    use crate::server::config::SslConfig;
    use crate::service::builder::ServiceBuilder;
    use crate::service::group::ServiceGroup;
    use std::sync::{Arc, atomic::Ordering};
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn clear_env() {
        // SAFETY: guarded by a process-wide mutex to avoid concurrent env mutation in tests.
        unsafe {
            std::env::remove_var("PORTFU_HOST");
            std::env::remove_var("PORTFU_PORT");
            std::env::remove_var("PORTFU_BACKLOG");
            std::env::remove_var("PORTFU_ACCEPTORS");
            std::env::remove_var("PORTFU_REUSEPORT");
            std::env::remove_var("PORTFU_SSL_ENABLED");
        }
    }

    #[test]
    fn from_env_keeps_defaults_when_env_is_missing() {
        let _guard = env_lock().lock().expect("failed to lock env mutex");
        clear_env();
        let builder = ServerBuilder::from_env();
        assert_eq!(builder.config.host, "localhost");
        assert_eq!(builder.config.port, 8080);
        assert_eq!(builder.config.backlog, 1024);
        assert!(builder.config.acceptors >= 1);
        assert!(builder.config.reuse_port);
        assert!(!builder.config.enable_ssl);
    }

    #[test]
    fn from_env_only_updates_values_that_exist() {
        let _guard = env_lock().lock().expect("failed to lock env mutex");
        clear_env();
        // SAFETY: guarded by a process-wide mutex to avoid concurrent env mutation in tests.
        unsafe {
            std::env::set_var("PORTFU_HOST", "0.0.0.0");
            std::env::set_var("PORTFU_PORT", "9090");
        }
        let builder = ServerBuilder::from_env();
        assert_eq!(builder.config.host, "0.0.0.0");
        assert_eq!(builder.config.port, 9090);
        assert_eq!(builder.config.backlog, 1024);
        assert!(builder.config.acceptors >= 1);
        assert!(builder.config.reuse_port);
        assert!(!builder.config.enable_ssl);
        clear_env();
    }

    #[test]
    fn from_env_parses_remaining_tunables_and_ignores_invalid_numbers() {
        let _guard = env_lock().lock().expect("failed to lock env mutex");
        clear_env();
        // SAFETY: guarded by a process-wide mutex to avoid concurrent env mutation in tests.
        unsafe {
            std::env::set_var("PORTFU_PORT", "not-a-port");
            std::env::set_var("PORTFU_BACKLOG", "2048");
            std::env::set_var("PORTFU_ACCEPTORS", "3");
            std::env::set_var("PORTFU_REUSEPORT", "false");
            std::env::set_var("PORTFU_SSL_ENABLED", "true");
        }
        let builder = ServerBuilder::from_env();
        assert_eq!(builder.config.port, 8080);
        assert_eq!(builder.config.backlog, 2048);
        assert_eq!(builder.config.acceptors, 3);
        assert!(!builder.config.reuse_port);
        assert!(builder.config.enable_ssl);
        clear_env();
    }

    #[test]
    fn service_group_preserves_registration_order() {
        let server = ServerBuilder::new()
            .service(ServiceBuilder::new("/first").name("first").build())
            .service_group(
                ServiceGroup::new()
                    .service(ServiceBuilder::new("/second").name("second").build())
                    .service(ServiceBuilder::new("/third").name("third").build()),
            )
            .service(ServiceBuilder::new("/fourth").name("fourth").build())
            .build();

        assert_eq!(
            server
                .services
                .iter()
                .map(|service| service.name())
                .collect::<Vec<_>>(),
            vec!["first", "second", "third", "fourth"]
        );
    }

    #[tokio::test]
    async fn service_group_merges_state_into_the_default_scope() {
        let server = ServerBuilder::new()
            .global_state("builder".to_string())
            .service_group(ServiceGroup::new().shared_state("first group".to_string()))
            .service_group(ServiceGroup::new().shared_state("last group".to_string()))
            .build();

        let state = server.scoped_state.read().await;
        assert_eq!(
            state
                .get("default")
                .and_then(|extensions| extensions.get::<Arc<String>>())
                .map(|value| value.as_str()),
            Some("last group")
        );
    }

    #[tokio::test]
    async fn fluent_builder_methods_store_config_and_scoped_state() {
        let ssl = SslConfig {
            domain: "example.test".to_string(),
            key: "key.pem".to_string(),
            certs: "cert.pem".to_string(),
            root_certs: "root.pem".to_string(),
        };
        let server = ServerBuilder::new()
            .host("0.0.0.0")
            .port(9090)
            .enable_ssl(true)
            .ssl_config(Some(ssl.clone()))
            .sni_ssl_config(ssl.clone())
            .global_state("global".to_string())
            .scoped_state("tenant", 99_u32)
            .build();

        assert_eq!(server.config.host, "0.0.0.0");
        assert_eq!(server.config.port, 9090);
        assert!(server.config.enable_ssl);
        assert_eq!(server.config.ssl_config, Some(ssl.clone()));
        assert_eq!(server.config.sni_ssl_configs, vec![ssl]);
        assert!(server.run.load(Ordering::Relaxed));

        let state = server.scoped_state.read().await;
        assert_eq!(
            state
                .get("default")
                .and_then(|ext| ext.get::<Arc<String>>())
                .map(|value| value.as_str()),
            Some("global")
        );
        assert_eq!(
            state
                .get("tenant")
                .and_then(|ext| ext.get::<Arc<u32>>())
                .map(|value| **value),
            Some(99)
        );
    }
}
