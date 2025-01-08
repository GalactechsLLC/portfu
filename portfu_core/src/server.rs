use crate::filters::{Filter, FilterFn, FilterResult};
use crate::service::{BodyType, IncomingRequest, Service, ServiceRequest, ServiceResponse};
use crate::signal::await_termination;
use crate::ssl::load_ssl_certs;
use crate::task::{Task, TaskFn};
use crate::wrappers::{WrapperFn, WrapperResult};
use crate::{IntoStreamBody, ServiceData, ServiceRegister, ServiceRegistry, StreamingBody};
use http::{Extensions, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::io::{Error, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::task::JoinSet;
use tokio::{select, spawn};
use tokio_rustls::TlsAcceptor;

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
    pub keep_alive: bool,
    pub half_close: bool,
    pub preserve_header_case: bool,
    pub max_buf_size: usize,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 8080,
            ssl_config: None,
            keep_alive: true,
            half_close: true,
            preserve_header_case: true,
            max_buf_size: 1024 * 1024 * 2, //2 Mib
        }
    }
}

#[derive(Debug)]
pub struct Server {
    pub registry: Arc<RwLock<ServiceRegistry>>,
    pub config: ServerConfig,
    pub run: Arc<AtomicBool>,
    pub shared_state: Arc<RwLock<Extensions>>,
    filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
}
impl Server {
    pub async fn run(self) -> Result<(), Error> {
        let server = Arc::new(self);
        {
            let slf = server.clone();
            server.shared_state.write().await.insert(slf);
        }
        let socket_addr = Self::get_socket_addr(&server.config)?;
        info!("Server Starting Up on {socket_addr}");
        let listener = TcpListener::bind(socket_addr).await?;
        let tls_acceptor = Arc::new(match server.config.ssl_config.as_ref() {
            Some(_) => {
                let certs = load_ssl_certs(&server.config)?;
                Some(TlsAcceptor::from(certs))
            }
            None => None,
        });
        let mut http = Builder::new();
        http.half_close(server.config.half_close);
        http.keep_alive(server.config.keep_alive);
        http.preserve_header_case(server.config.preserve_header_case);
        http.max_buf_size(server.config.max_buf_size);
        let http = Arc::new(http);
        let server_run_handle = server.run.clone();
        spawn(async move {
            let _ = await_termination().await;
            server_run_handle.store(false, Ordering::Relaxed);
        });
        let mut background_tasks = JoinSet::new();
        for task in server.registry.read().await.tasks.iter().cloned() {
            let state = server.shared_state.clone();
            info!("Spawning Task {}", task.name());
            background_tasks.spawn(async move {
                if let Err(e) = task.task_fn.run(state.clone()).await {
                    error!("Error in background task: {e:?}");
                }
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
                                                error!("Error serving tls connection: {:?}", err);
                                            }
                                        }
                                        Err(e) => {
                                            error!("Error accepting tls connection: {:?}", e);
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
        info!("Server Exiting");
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
    ) -> Result<Response<StreamingBody>, Error> {
        request.extensions_mut().insert(address);
        request.extensions_mut().insert(server.shared_state.clone()); //Put the Server Shared State in the Request Extensions
        let mut response: ServiceResponse = ServiceResponse::new();
        let handle = if !server.filters.is_empty() {
            let mut handle = true;
            for f in server.filters.iter() {
                if f.filter(&request).await != FilterResult::Allow {
                    handle = false;
                    break;
                }
            }
            handle
        } else {
            true
        };
        if !handle {
            *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
            Ok(response.into())
        } else {
            let mut handler = None;
            let services: Vec<Arc<Service>> = server.registry.read().await.services.to_vec();
            for service in services {
                if service.handles(&request).await {
                    handler = Some(service.clone());
                    break;
                }
            }
            match handler {
                Some(service) => handle_service(request, service, server.clone(), response).await,
                None => {
                    if let Some(service) = server.registry.read().await.default_service.clone() {
                        handle_service(request, service, server.clone(), response).await
                    } else {
                        *response.status_mut() = StatusCode::NOT_FOUND;
                        Ok(response.into())
                    }
                }
            }
        }
    }
}

pub async fn handle_service(
    mut request: Request<Incoming>,
    service: Arc<Service>,
    server: Arc<Server>,
    response: ServiceResponse,
) -> Result<Response<StreamingBody>, Error> {
    request
        .extensions_mut()
        .extend(service.shared_state.clone());
    let mut service_data = ServiceData {
        server: server.clone(),
        request: ServiceRequest {
            request: IncomingRequest::Stream(request.map(|b| b.stream_body())),
            path: service.path.clone(),
        },
        response,
    };
    for func in server.wrappers.iter() {
        match func.before(&mut service_data).await {
            WrapperResult::Continue => {}
            WrapperResult::Return => {
                return Ok(service_data.response.into());
            }
        }
    }
    service_data = service
        .handle(service_data)
        .await
        .unwrap_or_else(|(mut sd, e)| {
            error!(
                "Service Error when Handling {} - {e:?}",
                sd.request.request.uri()
            );
            *sd.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            sd.response
                .set_body(BodyType::Sized(Full::new(Bytes::from(format!("{:?}", e)))));
            sd
        });
    for func in server.wrappers.iter() {
        match func.after(&mut service_data).await {
            WrapperResult::Continue => {}
            WrapperResult::Return => {
                return Ok(service_data.response.into());
            }
        }
    }
    Ok(service_data.response.into())
}

pub struct ServerBuilder {
    services: ServiceRegistry,
    config: ServerConfig,
    shared_state: Extensions,
    run_handle: Arc<AtomicBool>,
    filters: Vec<Arc<dyn FilterFn + Sync + Send>>,
    wrappers: Vec<Arc<dyn WrapperFn + Sync + Send>>,
}
pub struct SharedState<T> {
    inner: Arc<T>,
}
impl<T> From<T> for SharedState<T> {
    fn from(value: T) -> Self {
        SharedState {
            inner: Arc::new(value),
        }
    }
}
impl<T> From<Arc<T>> for SharedState<T> {
    fn from(inner: Arc<T>) -> Self {
        SharedState { inner }
    }
}
impl ServerBuilder {
    pub fn from_config(config: ServerConfig) -> Self {
        Self {
            services: ServiceRegistry {
                services: vec![],
                tasks: vec![],
                default_service: None,
            },
            config,
            shared_state: Extensions::default(),
            run_handle: Arc::new(AtomicBool::new(true)),
            filters: vec![],
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
        service.register(&mut s.services, s.shared_state.clone());
        s
    }
    pub fn default_service(self, service: Service) -> Self {
        let mut s = self;
        s.services.default_service = Some(Arc::new(service));
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
        s.shared_state.insert(state.inner);
        s
    }
    pub fn build(self) -> Server {
        Server {
            registry: Arc::new(RwLock::new(self.services)),
            config: self.config,
            run: self.run_handle,
            shared_state: Arc::new(RwLock::new(self.shared_state)),
            filters: self.filters,
            wrappers: self.wrappers,
        }
    }
}
impl Default for ServerBuilder {
    fn default() -> Self {
        Self {
            services: ServiceRegistry {
                services: vec![],
                tasks: vec![],
                default_service: None,
            },
            config: ServerConfig::default(),
            shared_state: Extensions::default(),
            run_handle: Arc::new(AtomicBool::new(true)),
            filters: vec![],
            wrappers: vec![],
        }
    }
}
