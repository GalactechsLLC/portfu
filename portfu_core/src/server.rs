pub mod builder;
pub mod config;
mod ssl;
pub mod state;

use crate::runtime::thread::ServerThread;
use crate::server::config::ServerConfig;
use crate::services::body::BodyType;
use crate::services::request::{RequestType, ServiceRequest};
use crate::services::response::ServiceResponse;
use crate::services::Service;
use crate::utils::signal::await_termination;
use crate::{IntoStreamBody, ServiceData, ServiceRegistry, StreamingBody};
use http::{Extensions, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use log::{debug, error, info};
use ssl::load_ssl_certs;
use std::env;
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

#[derive(Debug)]
pub struct Server {
    pub registry: Arc<RwLock<ServiceRegistry>>,
    pub config: ServerConfig,
    pub run: Arc<AtomicBool>,
    pub shared_state: Arc<RwLock<Extensions>>,
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
        //Check for various ways of using SSL
        let tls_acceptor = if server.config.ssl_config.is_some()
            || (env::var("PRIVATE_CA_CRT").ok().is_some()
                && env::var("PRIVATE_CA_KEY").ok().is_some())
            || (env::var("SSL_CERTS").ok().is_some()
                && env::var("SSL_PRIVATE_KEY").ok().is_some()
                && env::var("SSL_ROOT_CERTS").ok().is_some())
        {
            let certs = load_ssl_certs(&server.config)?;
            Some(TlsAcceptor::from(certs))
        } else {
            None
        };
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
                if let Err(e) = task.handle.run(state.clone()).await {
                    error!("Error in background task: {e:?}");
                } else {
                    info!("Task Finished without Error: {}", task.name());
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
                                if let Some(acceptor) = tls_acceptor.as_ref() {
                                    match acceptor.accept(stream).await {
                                        Ok(stream) => {
                                            let service = service_fn(move |req| {
                                                let server = server.clone();
                                                Self::connection_handler(server, req, address)
                                            });
                                            let connection = http.serve_connection(TokioIo::new(stream), service).with_upgrades();
                                            if let Err(err) = connection.await {
                                                error!("Error serving tls connection: {err:?}");
                                            }
                                        }
                                        Err(e) => {
                                            error!("Error accepting tls connection: {e:?}");
                                        }
                                    }
                                } else {
                                    let service = service_fn(move |req| {
                                        let server = server.clone();
                                        Self::connection_handler(server, req, address)
                                    });
                                    let connection = http.serve_connection(TokioIo::new(stream), service).with_upgrades();
                                    if let Err(err) = connection.await {
                                        error!("Error serving connection: {err:?}");
                                    }
                                };
                            });
                        }
                        Err(e) => {
                            error!("Error accepting connection: {e:?}");
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
                    format!("Failed to parse Host: {e:?}"),
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
        request
            .extensions_mut()
            .extend(server.shared_state.read().await.clone()); //Put the Server Shared State in the Request Extensions
        let mut response: ServiceResponse = ServiceResponse::new();
        let mut handler = None;
        let services: Vec<Arc<Service>> = server.registry.read().await.services.to_vec();
        for service in services {
            if service.handles(&request).await {
                handler = Some(service.clone());
                break;
            }
        }
        debug!("got request: {}, {}", request.uri(), handler.is_some());
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
        request: ServiceRequest::new(
            RequestType::Stream(request.map(|b| b.stream_body())),
            service.path.clone(),
        ),
        response,
    };
    service_data = service
        .handle(service_data)
        .await
        .unwrap_or_else(|(mut sd, e)| {
            error!("Service Error when Handling {} - {e:?}", sd.request.uri());
            *sd.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            sd.response
                .set_body(BodyType::Sized(Full::new(Bytes::from(format!("{e:?}")))));
            sd
        });
    Ok(service_data.response.into())
}
