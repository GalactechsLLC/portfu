use std::io::{Error, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use log::error;
use tokio::net::TcpListener;
use portfu_core::{ServiceRegister, ServiceRegistry};
use serde::{Deserialize, Serialize};
use tokio::select;
use tokio_rustls::TlsAcceptor;
use portfu_core::data_map::DynMap;
use portfu_core::service::ServiceRequest;
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

#[derive(Debug)]
pub struct Server {
    pub services: Arc<ServiceRegistry>,
    pub config : ServerConfig,
    pub run: Arc<AtomicBool>
}
impl Server {
    pub async fn run(self) -> Result<(), Error> {
        let server = Arc::new(self);
        let socket_addr = Self::get_socket_addr(&server.config)?;
        let listener = TcpListener::bind(socket_addr).await?;
        let tls_acceptor = match server.config.ssl_config.as_ref() {
            Some(_) => {
                let certs = load_ssl_certs(&server.config)?;
                Some(TlsAcceptor::from(certs))
            }
            None => { None }
        };
        let mut http = Builder::new();
        http.keep_alive(true);
        while server.run.load(Ordering::Relaxed) {
            select!(
                res = listener.accept() => {
                    match res {
                        Ok((stream, address)) => {
                            let server = server.clone();
                            let service = service_fn(move |req| {
                                let server = server.clone();
                                Self::connection_handler(server, req, address.into())
                            });
                            if let Some(acceptor) = tls_acceptor.as_ref() {
                                match acceptor.accept(stream).await {
                                    Ok(stream) => {
                                        let connection = http.serve_connection(TokioIo::new(stream), service);
                                        tokio::spawn( async move {
                                            if let Err(err) = connection.await {
                                                error!("Error serving connection: {:?}", err);
                                            }
                                            Ok::<(), Error>(())
                                        });
                                    }
                                    Err(e) => {
                                        error!("Error accepting connection: {:?}", e);
                                        continue;
                                    }
                                }
                            } else {
                                let connection = http.serve_connection(TokioIo::new(stream), service);
                                tokio::spawn( async move {
                                    if let Err(err) = connection.await {
                                        error!("Error serving connection: {:?}", err);
                                    }
                                    Ok::<(), Error>(())
                                });
                            };
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
        req: Request<Incoming>,
        address: SocketAddr,
    ) -> Result<Response<Full<Bytes>>, Error> {
        let respnse = Response::new(Full::new(Bytes::new()));
        for service in server.services.services.iter().cloned() {
            if service.handles(&req) {
                let mut dyn_map = DynMap::new();
                dyn_map.insert(address);
                dyn_map.insert(address);
                dyn_map.insert(address);
                let service_request = ServiceRequest {
                    request: req,
                    path: service.path.clone(),
                    dyn_map
                };
                return match service.handle(&address, &service_request, respnse).await {
                    Ok(r) => {
                        Ok(r)
                    }
                    Err(respnse) => {
                        Ok(respnse)
                    }
                }
            }
        }
        Ok(respnse)
    }
}

pub struct ServerBuilder {
    services: ServiceRegistry,
    pub config : ServerConfig
}
impl<'a> ServerBuilder {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            services: ServiceRegistry{
                services: vec![]
            },
            config
        }
    }
    pub fn register<T: ServiceRegister>(self, service: T) -> Self {
        let mut s = self;
        service.register(&mut s.services);
        s
    }
    pub fn build(self) -> Server {
        Server {
            services: Arc::new(self.services),
            config: self.config,
            run: Arc::new(AtomicBool::new(true))
        }
    }
}