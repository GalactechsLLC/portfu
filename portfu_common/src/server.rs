pub mod builder;
pub mod config;
pub mod state;

use crate::error::PortfuError;
use crate::router::route::Route;
use crate::server::config::ServerConfig;
use crate::service::request::{Request, RequestType};
use crate::service::response::Response;
use crate::service::{Service, StreamingBody};
use crate::signal::await_termination;
use crate::stream::IntoStreamBody;
use http::Extensions;
use hyper::body::Incoming;
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use log::{error, info, warn};
use once_cell::sync::Lazy;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::{RwLock, watch};
use tokio::{select, spawn};

pub trait ServiceRegister: Send + Sync {
    fn register(self, registry: &mut ServiceRegistry);
}

pub struct ServiceRegistration {
    pub register: fn(registry: &mut ServiceRegistry) -> Service,
}

inventory::collect!(ServiceRegistration);

#[derive(Clone, Default)]
pub struct ServiceRegistry {
    pub services: Vec<Service>,
    pub default_service: Option<Service>,
}

pub static SERVICE_REGISTRY: Lazy<Arc<ServiceRegistry>> = Lazy::new(|| Arc::new(load_registry()));
static DEFAULT_ROUTE: Lazy<Arc<Route>> = Lazy::new(|| Arc::new(Route::new("/".to_string())));

fn load_registry() -> ServiceRegistry {
    let mut registry = ServiceRegistry::default();
    let services: Vec<Service> = inventory::iter::<ServiceRegistration>
        .into_iter()
        .map(|reg| (reg.register)(&mut registry))
        .collect();
    registry.services.extend(services);
    registry
}

#[derive(Default)]
pub struct Server {
    pub run: Arc<AtomicBool>,
    pub config: ServerConfig,
    pub global_state: Arc<RwLock<Extensions>>,
    pub default_service: Option<Service>,
}
impl Server {
    pub async fn run(self) -> Result<(), PortfuError> {
        let server = Arc::new(self);
        {
            let slf = server.clone();
            server.global_state.write().await.insert(slf);
        }
        let socket_addr = SocketAddr::from((
            server
                .config
                .host
                .as_str()
                .parse::<std::net::IpAddr>()
                .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
            server.config.port,
        ));
        info!("Server Starting Up on {socket_addr}");
        let backlog = server.config.backlog.max(128);
        let mut acceptors = server.config.acceptors.max(1);
        if acceptors > 1 && !server.config.reuse_port {
            warn!(
                "PORTFU_ACCEPTORS is set but reuse_port is disabled; falling back to a single acceptor"
            );
            acceptors = 1;
        }
        info!("Using {} Acceptors", acceptors);
        let mut listeners = Vec::with_capacity(acceptors);
        for _ in 0..acceptors {
            listeners.push(Self::bind_listener(
                socket_addr,
                backlog,
                server.config.reuse_port,
            )?);
        }
        let mut http = Builder::new();
        http.half_close(server.config.half_close);
        http.keep_alive(server.config.keep_alive);
        http.preserve_header_case(server.config.preserve_header_case);
        http.max_buf_size(server.config.max_buf_size);
        let http = Arc::new(http);
        let server_run_handle = server.run.clone();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shutdown_tx_handle = shutdown_tx.clone();
        spawn(async move {
            let _ = await_termination().await;
            server_run_handle.store(false, Ordering::Relaxed);
            let _ = shutdown_tx_handle.send(true);
        });
        let mut acceptor_handles = Vec::with_capacity(listeners.len());
        for listener in listeners {
            let server = server.clone();
            let http = http.clone();
            let mut shutdown_rx = shutdown_rx.clone();
            acceptor_handles.push(spawn(async move {
                loop {
                    select!(
                        _ = shutdown_rx.changed() => {
                            break;
                        }
                        stream_res = listener.accept() => {
                            match stream_res {
                                Ok((stream, address)) => {
                                    let server = server.clone();
                                    let http = http.clone();
                                    spawn(async move {
                                        let service = service_fn(move |req| {
                                            let server = server.clone();
                                            Self::connection_handler(server, req, address)
                                        });
                                        let connection = http.serve_connection(TokioIo::new(stream), service).with_upgrades();
                                        if let Err(err) = connection.await {
                                            error!("Error serving connection: {err:?}");
                                        }
                                    });
                                }
                                Err(e) => {
                                    if !*shutdown_rx.borrow() {
                                        error!("Error accepting connection: {e:?}");
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    )
                }
            }));
        }
        let mut shutdown_rx = shutdown_rx.clone();
        let _ = shutdown_rx.changed().await;
        info!("Got Shutdown Signal");
        for handle in acceptor_handles {
            let _ = handle.await;
        }
        info!("Server Exiting");
        Ok(())
    }

    fn bind_listener(
        socket_addr: SocketAddr,
        backlog: u32,
        reuse_port: bool,
    ) -> Result<TcpListener, PortfuError> {
        let socket = match socket_addr {
            SocketAddr::V4(_) => TcpSocket::new_v4(),
            SocketAddr::V6(_) => TcpSocket::new_v6(),
        }
        .map_err(PortfuError::Io)?;
        socket.set_reuseaddr(true).map_err(PortfuError::Io)?;
        if reuse_port {
            #[cfg(unix)]
            {
                socket.set_reuseport(true).map_err(PortfuError::Io)?;
            }
        }
        socket.bind(socket_addr).map_err(PortfuError::Io)?;
        socket.listen(backlog).map_err(PortfuError::Io)
    }

    #[inline]
    async fn connection_handler(
        server: Arc<Self>,
        mut request: http::Request<Incoming>,
        address: SocketAddr,
    ) -> Result<http::Response<StreamingBody>, PortfuError> {
        request.extensions_mut().insert(address);
        let global_state = server.global_state.read().await.clone();
        request.extensions_mut().extend(global_state);
        let mut request = Request::new(
            RequestType::Stream(request.map(|b| b.stream_body())),
            DEFAULT_ROUTE.clone(),
        );
        for service in &SERVICE_REGISTRY.services {
            if service.serves(&request).await {
                *request.route_mut() = service.route().clone();
                return service.serve(&mut request).await.map(Into::into);
            }
        }
        match &server.default_service {
            Some(service) => service.serve(&mut request).await.map(Into::into),
            None => Ok(Response::not_found("Failed to find service for request").into()),
        }
    }
}
