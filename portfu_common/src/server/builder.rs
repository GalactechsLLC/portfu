use crate::server::Server;
use crate::server::config::{ServerConfig, SslConfig};
use crate::server::state::SharedState;
use crate::service::Service;
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
    pub default_service: Option<Service>,
    pub health_service: Option<Service>,
}
impl ServerBuilder {
    pub fn from_env() -> Self {
        let mut builder = ServerBuilder {
            run: Arc::new(AtomicBool::new(true)),
            config: ServerConfig::default(),
            scoped_state: HashMap::new(),
            default_service: None,
            health_service: None,
        };
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
    pub fn build(self) -> Server {
        Server {
            run: self.run,
            config: self.config,
            scoped_state: Arc::new(RwLock::new(self.scoped_state)),
            default_service: self.default_service,
            health_service: self.health_service,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ServerBuilder;
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
}
