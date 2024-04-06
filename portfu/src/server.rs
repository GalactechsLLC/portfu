use crate::signal::await_termination;
use crate::ssl::load_ssl_certs;
use crate::pfcore::filters::FilterFn;
use crate::pfcore::filters::{Filter, FilterResult};
use crate::pfcore::wrappers::{WrapperFn, WrapperResult};
use crate::pfcore::service::{IncomingRequest, ServiceRequest};
use crate::pfcore::{ServiceRegister, ServiceRegistry, ServiceData};
use http::{Extensions, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use log::error;
use serde::{Deserialize, Serialize};
use std::io::{Error, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::{select, spawn};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use pfcore::task::Task;

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SslConfig {
    pub domain: String,
    pub key: String,
    pub certs: String,
    pub root_certs: String,
}

#[derive(Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub ssl_config: Option<SslConfig>,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 8080,
            ssl_config: None,
        }
    }
}

#[derive(Debug)]
pub struct Server {
    pub services: Arc<ServiceRegistry>,
    pub config: ServerConfig,
    pub run: Arc<AtomicBool>,
    pub shared_state: Extensions,
    filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    tasks: Vec<Arc<Task>>,
    wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
}
impl Server {
    pub async fn run(self) -> Result<(), Error> {
        let server = Arc::new(self);
        let socket_addr = Self::get_socket_addr(&server.config)?;
        let listener = TcpListener::bind(socket_addr).await?;
        let tls_acceptor = Arc::new(match server.config.ssl_config.as_ref() {
            Some(_) => {
                let certs = load_ssl_certs(&server.config)?;
                Some(TlsAcceptor::from(certs))
            }
            None => None,
        });
        let mut http = Builder::new();
        http.keep_alive(true);
        let http = Arc::new(http);
        let server_run_handle = server.run.clone();
        spawn(async move {
            let _ = await_termination().await;
            server_run_handle.store(false, Ordering::Relaxed);
        });
        let mut background_tasks = JoinSet::new();
        for task in server.tasks.iter().cloned() {
            let state = server.shared_state.clone();
            background_tasks.spawn(async move {
                task.task_fn.run(state).await
            });
        }
        while server.run.load(Ordering::Relaxed) {
            select!(
                res = listener.accept() => {
                    match res {
                        Ok((stream, address)) => {
                            let server = server.clone();
                            let tls_acceptor = tls_acceptor.clone();
                            let http = http.clone();
                            spawn(async move {
                                let service = service_fn(move |req| {
                                    let server = server.clone();
                                    Self::connection_handler(server, req, address)
                                });
                                if let Some(acceptor) = tls_acceptor.as_ref() {
                                    match acceptor.accept(stream).await {
                                        Ok(stream) => {
                                            let connection = http.serve_connection(TokioIo::new(stream), service).with_upgrades();
                                            if let Err(err) = connection.await {
                                                error!("Error serving connection: {:?}", err);
                                            }
                                        }
                                        Err(e) => {
                                            error!("Error accepting connection: {:?}", e);
                                        }
                                    }
                                } else {
                                    let connection = http.serve_connection(TokioIo::new(stream), service).with_upgrades();
                                    if let Err(err) = connection.await {
                                        error!("Error serving connection: {:?}", err);
                                    }
                                };
                            });
                        }
                        Err(e) => {
                            error!("Error accepting connection: {:?}", e);
                        }
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            )
        }
        background_tasks.shutdown().await;
        Ok(())
    }

    fn get_socket_addr(config: &ServerConfig) -> Result<SocketAddr, Error> {
        Ok(SocketAddr::from((
            Ipv4Addr::from_str(if config.host == "localhost" {
                "127.0.0.1"
            } else {
                &config.host
            })
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("Failed to parse Host: {:?}", e),
                )
            })?,
            config.port,
        )))
    }

    #[inline]
    async fn connection_handler(
        server: Arc<Self>,
        mut request: Request<Incoming>,
        address: SocketAddr,
    ) -> Result<Response<Full<Bytes>>, Error> {
        let mut response: Response<Full<Bytes>> = Default::default();
        if !server
            .filters.is_empty() && !server
            .filters
            .iter()
            .cloned()
            .all(|f| f.filter(&request) == FilterResult::Allow) {
            *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
            Ok(response)
        } else {
            match server.services.services.iter().find(|s| s.handles(&request)).cloned() {
                Some(service) => {
                    request.extensions_mut().extend(server.shared_state.clone());
                    let mut service_data = ServiceData {
                        request: ServiceRequest {
                            request: IncomingRequest::Stream(request),
                            path: service.path.clone(),
                        },
                        response,
                    };
                    service_data.request.insert(address);
                    for func in server.wrappers.iter() {
                        match func.before(&mut service_data).await {
                            WrapperResult::Continue => {},
                            WrapperResult::Return => {
                                return Ok(service_data.response);
                            }
                        }
                    }
                    service.handle(&mut service_data).await?;
                    for func in server.wrappers.iter() {
                        match func.after(&mut service_data).await {
                            WrapperResult::Continue => {},
                            WrapperResult::Return => {
                                return Ok(service_data.response);
                            }
                        }
                    }
                    Ok(service_data.response)
                }
                None => {
                    *response.status_mut() = StatusCode::NOT_FOUND;
                    Ok(response)
                }
            }
        }
    }
}

pub struct ServerBuilder {
    services: ServiceRegistry,
    config: ServerConfig,
    shared_state: Extensions,
    filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    tasks: Vec<Arc<Task>>,
    wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
}
impl ServerBuilder {
    pub fn from_config(config: ServerConfig) -> Self {
        Self {
            services: ServiceRegistry { services: vec![] },
            config,
            shared_state: Extensions::default(),
            filters: vec![],
            tasks: vec![],
            wrappers: vec![],
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
        service.register(&mut s.services);
        s
    }
    pub fn filter(self, filter: Filter) -> Self {
        let mut s = self;
        s.filters.push(Arc::new(filter));
        s
    }
    pub fn wrap(self, wrapper: Arc<dyn WrapperFn + Sync + Send>) -> Self {
        let mut s = self;
        s.wrappers.push(wrapper);
        s
    }
    pub fn task<T: Into<Task>>(mut self, task: T) -> Self {
        self.tasks.push(Arc::new(task.into()));
        self
    }
    pub fn shared_state<T: Send + Sync + 'static>(self, shared_state: T) -> Self {
        let mut s = self;
        s.shared_state.insert(Arc::new(shared_state));
        s
    }
    pub fn build(self) -> Server {
        Server {
            services: Arc::new(self.services),
            config: self.config,
            run: Arc::new(AtomicBool::new(true)),
            shared_state: self.shared_state,
            filters: self.filters,
            tasks: self.tasks,
            wrappers: self.wrappers,
        }
    }
}
impl Default for ServerBuilder {
    fn default() -> Self {
        Self {
            services: ServiceRegistry { services: vec![] },
            config: ServerConfig::default(),
            shared_state: Extensions::default(),
            filters: vec![],
            tasks: vec![],
            wrappers: vec![],
        }
    }
}
