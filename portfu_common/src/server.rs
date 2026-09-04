pub mod builder;
pub mod config;
pub mod connection;
pub(crate) mod runtime;
#[cfg(feature = "tls")]
pub(crate) mod ssl;
pub mod state;

use crate::error::PortfuError;
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::router::route::Route;
use crate::server::config::ServerConfig;
use crate::server::connection::ConnectionInfo;
use crate::server::runtime::ServerRuntime;
#[cfg(feature = "tls")]
use crate::server::ssl::{load_ssl_certs, negotiated_tls_version};
use crate::service::request::{Request, RequestType};
use crate::service::response::Response;
use crate::service::{Service, StreamingBody};
use crate::signal::TerminationSignals;
use crate::stream::IntoStreamBody;
#[cfg(feature = "websocket")]
use crate::websocket::WebSocketAdmissionMiddleware;
use http::Extensions;
use hyper::body::Incoming;
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use log::{error, info, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::{RwLock, watch};
use tokio::{select, spawn};
#[cfg(feature = "tls")]
use tokio_rustls::TlsAcceptor;

pub trait ServiceRegister: Send + Sync {
    fn register(self, registry: &mut ServiceRegistry);
}

pub struct ServiceRegistration {
    pub register: fn(registry: &mut ServiceRegistry) -> Service,
}

inventory::collect!(ServiceRegistration);

pub type TaskFuture = Pin<Box<dyn Future<Output = Result<(), PortfuError>> + Send + 'static>>;

pub struct TaskRegistration {
    pub run: fn(server: Arc<Server>) -> TaskFuture,
}

inventory::collect!(TaskRegistration);

#[derive(Clone)]
pub struct ServerHandle {
    run: Arc<AtomicBool>,
    shutdown: watch::Sender<bool>,
    runtime: Arc<ServerRuntime>,
}

impl ServerHandle {
    /// Stops admission and accepting, then drains tracked work for the configured grace period.
    pub fn shutdown(&self) {
        self.run.store(false, Ordering::Relaxed);
        self.runtime.begin_shutdown();
        self.shutdown.send_replace(true);
    }

    /// Cancels all tracked work immediately without terminating the process.
    pub fn force_shutdown(&self) {
        self.run.store(false, Ordering::Relaxed);
        self.runtime.force_shutdown();
        self.shutdown.send_replace(true);
    }
}

#[derive(Clone, Default)]
pub struct ServiceRegistry {
    pub services: Vec<Service>,
    pub default_service: Option<Service>,
}

pub static SERVICE_REGISTRY: Lazy<Arc<ServiceRegistry>> = Lazy::new(|| Arc::new(load_registry()));
static DEFAULT_ROUTE: Lazy<Arc<Route>> = Lazy::new(|| Arc::new(Route::new("/".to_string())));
pub(crate) const DEFAULT_SCOPE: &str = "default";

fn load_registry() -> ServiceRegistry {
    let mut registry = ServiceRegistry::default();
    let services: Vec<Service> = inventory::iter::<ServiceRegistration>
        .into_iter()
        .map(|reg| (reg.register)(&mut registry))
        .collect();
    registry.services.extend(services);
    registry
}

pub struct Server {
    pub run: Arc<AtomicBool>,
    pub config: ServerConfig,
    pub scoped_state: Arc<RwLock<HashMap<String, Extensions>>>,
    pub services: Vec<Service>,
    pub middleware: Vec<Arc<dyn Middleware + Send + Sync>>,
    pub default_service: Option<Service>,
    pub(crate) shutdown: watch::Sender<bool>,
    pub(crate) runtime: Arc<ServerRuntime>,
    #[cfg(feature = "websocket")]
    pub(crate) websocket_admission: Vec<Arc<dyn WebSocketAdmissionMiddleware + Send + Sync>>,
}
impl Server {
    pub fn handle(&self) -> ServerHandle {
        ServerHandle {
            run: self.run.clone(),
            shutdown: self.shutdown.clone(),
            runtime: self.runtime.clone(),
        }
    }

    pub async fn run(self) -> Result<(), PortfuError> {
        let server = Arc::new(self);
        {
            let slf = server.clone();
            server
                .scoped_state
                .write()
                .await
                .entry(DEFAULT_SCOPE.to_string())
                .or_default()
                .insert(slf);
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
        http.timer(TokioTimer::new());
        http.header_read_timeout(server.config.http_header_read_timeout);
        let http = Arc::new(http);
        #[cfg(feature = "tls")]
        let loaded_tls = if server.config.tls.is_some()
            || (env::var("SSL_CERTS").ok().is_some() && env::var("SSL_PRIVATE_KEY").ok().is_some())
        {
            Some(load_ssl_certs(&server.config)?)
        } else {
            None
        };
        #[cfg(feature = "tls")]
        let tls_acceptor = loaded_tls
            .as_ref()
            .map(|tls| TlsAcceptor::from(tls.server_config.clone()));
        #[cfg(feature = "tls")]
        let client_verifier = loaded_tls
            .as_ref()
            .and_then(|tls| tls.client_verifier.clone());
        #[cfg(feature = "tls")]
        let tls_handshake_timeout = server
            .config
            .tls
            .as_ref()
            .map(|tls| tls.handshake_timeout)
            .unwrap_or_else(|| crate::server::config::TlsConfig::default().handshake_timeout);
        #[cfg(not(feature = "tls"))]
        if server.config.tls.is_some()
            || (env::var("SSL_CERTS").ok().is_some() && env::var("SSL_PRIVATE_KEY").ok().is_some())
        {
            return Err(PortfuError::Internal(
                "TLS support requires the `tls` feature".to_string(),
            ));
        }
        let shutdown_handle = server.handle();
        let shutdown_rx = server.shutdown.subscribe();
        let signal_task = spawn(async move {
            match TerminationSignals::new() {
                Ok(mut signals) => {
                    signals.recv().await;
                    shutdown_handle.shutdown();
                    signals.recv().await;
                    error!("Received a second shutdown signal; forcing process termination");
                    shutdown_handle.force_shutdown();
                    std::process::exit(130);
                }
                Err(error) => error!("Failed to install shutdown signal handlers: {error}"),
            }
        });
        let mut acceptor_handles = Vec::with_capacity(listeners.len());
        for task in inventory::iter::<TaskRegistration> {
            let server = server.clone();
            let runtime = server.runtime.clone();
            runtime.spawn_background(async move {
                if let Err(e) = (task.run)(server).await {
                    error!("Background task failed: {e:?}");
                }
            });
        }
        for listener in listeners {
            let server = server.clone();
            let http = http.clone();
            #[cfg(feature = "tls")]
            let tls_acceptor = tls_acceptor.clone();
            #[cfg(feature = "tls")]
            let client_verifier = client_verifier.clone();
            let mut shutdown_rx = shutdown_rx.clone();
            acceptor_handles.push(spawn(async move {
                loop {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                    select!(
                        _ = shutdown_rx.changed() => {
                            break;
                        }
                        stream_res = listener.accept() => {
                            match stream_res {
                                Ok((stream, address)) => {
                                    let server = server.clone();
                                    let http = http.clone();
                                    let local_address = stream.local_addr().unwrap_or(socket_addr);
                                    #[cfg(feature = "tls")]
                                    let tls_acceptor = tls_acceptor.clone();
                                    #[cfg(feature = "tls")]
                                    let client_verifier = client_verifier.clone();
                                    #[cfg(feature = "tls")]
                                    let tls_handshake_timeout = tls_handshake_timeout;
                                    let cancellation = server.runtime.cancellation();
                                    let runtime = server.runtime.clone();
                                    runtime.spawn_http(async move {
                                        #[cfg(feature = "tls")]
                                        if let Some(acceptor) = tls_acceptor.as_ref() {
                                            let stream = tokio::select! {
                                                _ = cancellation.cancelled() => return,
                                                result = tokio::time::timeout(tls_handshake_timeout, acceptor.accept(stream)) => {
                                                    match result {
                                                        Ok(result) => result,
                                                        Err(_) => {
                                                            warn!("TLS handshake timed out for {address}");
                                                            return;
                                                        }
                                                    }
                                                },
                                            };
                                            match stream {
                                                Ok(stream) => {
                                                    let (_, tls_connection) = stream.get_ref();
                                                    let mut connection_info = ConnectionInfo::plaintext(address, local_address);
                                                    connection_info.tls_version = tls_connection
                                                        .protocol_version()
                                                        .and_then(negotiated_tls_version);
                                                    connection_info.client_identity = client_verifier
                                                        .as_ref()
                                                        .and_then(|verifier| tls_connection.peer_certificates().and_then(|certs| verifier.identity(certs)));
                                                    let service = service_fn(move |req| {
                                                        let server = server.clone();
                                                        let connection_info = connection_info.clone();
                                                        Self::connection_handler(server, req, connection_info)
                                                    });
                                                    let connection = http.serve_connection(TokioIo::new(stream), service).with_upgrades();
                                                    tokio::pin!(connection);
                                                    tokio::select! {
                                                        result = &mut connection => {
                                                            if let Err(err) = result {
                                                                error!("Error serving tls connection: {err:?}");
                                                            }
                                                        }
                                                        _ = cancellation.cancelled() => {
                                                            connection.as_mut().graceful_shutdown();
                                                            if let Err(err) = connection.await {
                                                                error!("Error draining tls connection: {err:?}");
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    error!("Error accepting tls connection: {e:?}");
                                                }
                                            }
                                            return;
                                        }
                                        let service = service_fn(move |req| {
                                            let server = server.clone();
                                            let connection_info = ConnectionInfo::plaintext(address, local_address);
                                            Self::connection_handler(server, req, connection_info)
                                        });
                                        let connection = http.serve_connection(TokioIo::new(stream), service).with_upgrades();
                                        tokio::pin!(connection);
                                        tokio::select! {
                                            result = &mut connection => {
                                                if let Err(err) = result {
                                                    error!("Error serving connection: {err:?}");
                                                }
                                            }
                                            _ = cancellation.cancelled() => {
                                                connection.as_mut().graceful_shutdown();
                                                if let Err(err) = connection.await {
                                                    error!("Error draining connection: {err:?}");
                                                }
                                            }
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
        if !*shutdown_rx.borrow() {
            let _ = shutdown_rx.changed().await;
        }
        info!("Got Shutdown Signal");
        for handle in acceptor_handles {
            let _ = handle.await;
        }
        if !server
            .runtime
            .drain(server.config.shutdown_grace_period)
            .await
        {
            warn!("Timed out while draining server connections and tasks");
        }
        signal_task.abort();
        let _ = signal_task.await;
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
        request: http::Request<Incoming>,
        connection_info: ConnectionInfo,
    ) -> Result<http::Response<StreamingBody>, PortfuError> {
        let scoped_state = server.scoped_state.read().await.clone();
        let mut request = Request::new(
            RequestType::Stream(request.map(|b| b.stream_body())),
            DEFAULT_ROUTE.clone(),
        );
        Self::set_request_scope_state(&mut request, &scoped_state, DEFAULT_SCOPE, &connection_info);
        for service in &server.services {
            Self::set_request_scope_state(
                &mut request,
                &scoped_state,
                service.scope(),
                &connection_info,
            );
            if service.serves(&request).await {
                *request.route_mut() = service.route().clone();
                return Self::serve_with_global_middleware(&server, service, &mut request)
                    .await
                    .map(Into::into);
            }
        }
        for service in &SERVICE_REGISTRY.services {
            Self::set_request_scope_state(
                &mut request,
                &scoped_state,
                service.scope(),
                &connection_info,
            );
            if service.serves(&request).await {
                *request.route_mut() = service.route().clone();
                return Self::serve_with_global_middleware(&server, service, &mut request)
                    .await
                    .map(Into::into);
            }
        }
        match &server.default_service {
            Some(service) => {
                Self::set_request_scope_state(
                    &mut request,
                    &scoped_state,
                    service.scope(),
                    &connection_info,
                );
                Self::serve_with_global_middleware(&server, service, &mut request)
                    .await
                    .map(Into::into)
            }
            None => Self::finalize_with_global_middleware(
                &server.middleware,
                &request,
                Response::not_found("Failed to find service for request"),
            )
            .await
            .map(Into::into),
        }
    }

    async fn serve_with_global_middleware(
        server: &Arc<Self>,
        service: &Service,
        request: &mut Request,
    ) -> Result<Response, PortfuError> {
        for middleware in &server.middleware {
            match middleware.before(request).await? {
                MiddlewareResult::Continue => {}
                MiddlewareResult::Return(response) => {
                    return Self::finalize_with_global_middleware(
                        &server.middleware,
                        request,
                        response,
                    )
                    .await;
                }
            }
        }

        let response = service.serve(request).await?;
        Self::finalize_with_global_middleware(&server.middleware, request, response).await
    }

    async fn finalize_with_global_middleware(
        middleware: &[Arc<dyn Middleware + Send + Sync>],
        request: &Request,
        mut response: Response,
    ) -> Result<Response, PortfuError> {
        for middleware in middleware {
            match middleware
                .after_with_request(request, &mut response)
                .await?
            {
                MiddlewareResult::Continue => {}
                MiddlewareResult::Return(response) => return Ok(response),
            }
        }
        Ok(response)
    }

    pub(crate) fn set_request_scope_state(
        request: &mut Request,
        scoped_state: &HashMap<String, Extensions>,
        scope: &str,
        connection_info: &ConnectionInfo,
    ) {
        if let Some(extensions) = request.shared_state_mut() {
            let upgrade = extensions.remove::<hyper::upgrade::OnUpgrade>();
            let mut scoped_extensions = Self::scope_state(scoped_state, scope);
            scoped_extensions.insert(connection_info.peer_addr);
            scoped_extensions.insert(connection_info.clone());
            if let Some(identity) = &connection_info.client_identity {
                scoped_extensions.insert(identity.clone());
            }
            if let Some(upgrade) = upgrade {
                scoped_extensions.insert(upgrade);
            }
            *extensions = scoped_extensions;
        }
    }

    fn scope_state(scoped_state: &HashMap<String, Extensions>, scope: &str) -> Extensions {
        let mut extensions = scoped_state.get(DEFAULT_SCOPE).cloned().unwrap_or_default();
        if scope != DEFAULT_SCOPE
            && let Some(scope_extensions) = scoped_state.get(scope)
        {
            extensions.extend(scope_extensions.clone());
        }
        extensions
    }
}

impl Default for Server {
    fn default() -> Self {
        crate::server::builder::ServerBuilder::new().build()
    }
}
