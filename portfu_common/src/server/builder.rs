use crate::server::Server;
use crate::server::config::ServerConfig;
use crate::server::state::SharedState;
use crate::service::Service;
use http::Extensions;
use std::env;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct ServerBuilder {
    pub run: Arc<AtomicBool>,
    pub config: ServerConfig,
    pub global_state: Extensions,
    pub default_service: Option<Service>,
}
impl ServerBuilder {
    pub fn from_env() -> Self {
        let _host = env::var("PORTFU_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let _port = env::var("PORTFU_PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .unwrap_or(8080);
        let _backlog = env::var("PORTFU_BACKLOG")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1024);
        let _acceptors = env::var("PORTFU_ACCEPTORS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1);
        let _reuse_port = env::var("PORTFU_REUSEPORT")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        ServerBuilder {
            run: Arc::new(AtomicBool::new(true)),
            config: ServerConfig::default(),
            global_state: Extensions::new(),
            default_service: None,
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
    pub fn global_state<S: Send + Sync + 'static>(
        mut self,
        value: impl Into<SharedState<S>>,
    ) -> Self {
        let state: SharedState<S> = value.into();
        self.global_state.insert::<Arc<S>>(state.into());
        self
    }
    pub fn build(self) -> Server {
        Server {
            run: self.run,
            config: self.config,
            global_state: Arc::new(RwLock::new(self.global_state)),
            default_service: self.default_service,
        }
    }
}
