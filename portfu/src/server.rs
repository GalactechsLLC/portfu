use std::io::{Error, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use http::{Extensions, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use log::error;
use tokio::net::TcpListener;
use portfu_core::{ServiceRegister, ServiceRegistry};
use serde::{Deserialize, Serialize};
use tokio::{select, spawn};
use tokio_rustls::TlsAcceptor;
use portfu_core::service::{IncomingRequest, ServiceRequest};
use crate::signal::await_termination;
use crate::ssl::load_ssl_certs;

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
    pub ssl_config: Option<SslConfig>
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
    pub config : ServerConfig,
    pub run: Arc<AtomicBool>,
    pub shared_state: Extensions
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
            None => { None }
        });
        let mut http = Builder::new();
        http.keep_alive(true);
        let http = Arc::new(http);
        let server_run_handle = server.run.clone();
        spawn(async move {
            let _ = await_termination().await;
            server_run_handle.store(false, Ordering::Relaxed);
        });
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
                                    Self::connection_handler(server, req, address.into())
                                });
                                if let Some(acceptor) = tls_acceptor.as_ref() {
                                    match acceptor.accept(stream).await {
                                        Ok(stream) => {
                                            let connection = http.serve_connection(TokioIo::new(stream), service);
                                            if let Err(err) = connection.await {
                                                error!("Error serving connection: {:?}", err);
                                            }
                                        }
                                        Err(e) => {
                                            error!("Error accepting connection: {:?}", e);
                                        }
                                    }
                                } else {
                                    let connection = http.serve_connection(TokioIo::new(stream), service);
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
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            )
        }
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

    async fn connection_handler(
        server: Arc<Self>,
        mut request: Request<Incoming>,
        address: SocketAddr,
    ) -> Result<Response<Full<Bytes>>, Error> {
        let mut response = Response::builder().status(StatusCode::NOT_FOUND).body(Full::new(Bytes::new())).unwrap();
        for service in server.services.services.iter().cloned() {
            if service.handles(&request) {
                *response.status_mut() = StatusCode::OK;
                request.extensions_mut().extend(server.shared_state.clone());
                let mut request = ServiceRequest {
                    request: IncomingRequest::Stream(request),
                    path: service.path.clone(),
                };
                request.insert(address);
                return Ok(match service.handle(request, response).await {
                    Ok(r) => {
                        r.response
                    }
                    Err(e) => {
                        e.response
                    }
                });
            }
        }
        Ok(response)
    }
}

pub struct ServerBuilder {
    services: ServiceRegistry,
    pub config : ServerConfig,
    pub shared_state: Extensions
}
impl ServerBuilder {
    pub fn new() -> Self {
        Self {
            services: ServiceRegistry{
                services: vec![]
            },
            config: ServerConfig::default(),
            shared_state: Extensions::default(),
        }
    }
    pub fn from_config(config: ServerConfig) -> Self {
        Self {
            services: ServiceRegistry{
                services: vec![]
            },
            config,
            shared_state: Extensions::default(),
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
        }
    }
}