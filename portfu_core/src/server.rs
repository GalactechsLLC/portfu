use std::env;
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
use sha2::{Digest, Sha256, Sha256VarCore};
use sha2::digest::Output;
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
    pub client_ssl_config: Option<SslConfig>,
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
            client_ssl_config: None,
            keep_alive: true,
            half_close: true,
            preserve_header_case: true,
            max_buf_size: 1024 * 1024 * 2, //2 Mib
        }
    }
}

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct PeerId(pub [u8; 32]);

pub fn peer_hash(input: impl AsRef<[u8]>) -> PeerId {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let mut buf = [0u8; 32];
    hasher.finalize_into(<&mut Output<Sha256VarCore>>::from(&mut buf));
    PeerId(buf)
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
        if server.config.ssl_config.is_none() &&
            env::var("PRIVATE_CA_CRT").ok().is_none() &&
            env::var("PRIVATE_CA_KEY").ok().is_none() &&
            env::var("SSL_CERTS").ok().is_none() &&
            env::var("SSL_PRIVATE_KEY").ok().is_none() &&
            env::var("SSL_ROOT_CERTS").ok().is_none() {
            env::set_var("PRIVATE_CA_CRT",
            r#"-----BEGIN CERTIFICATE-----
MIIDKTCCAhGgAwIBAgIUEwvVHT/nnEbmFRRPvEhTbO0FwwswDQYJKoZIhvcNAQEL
BQAwRDENMAsGA1UECgwEQ2hpYTEQMA4GA1UEAwwHQ2hpYSBDQTEhMB8GA1UECwwY
T3JnYW5pYyBGYXJtaW5nIERpdmlzaW9uMB4XDTIzMTIxNzAxNTY1MloXDTMzMTIx
NDAxNTY1MlowRDENMAsGA1UECgwEQ2hpYTEQMA4GA1UEAwwHQ2hpYSBDQTEhMB8G
A1UECwwYT3JnYW5pYyBGYXJtaW5nIERpdmlzaW9uMIIBIjANBgkqhkiG9w0BAQEF
AAOCAQ8AMIIBCgKCAQEAmSVb4oXGe9HIenJKmy7XcSBJlISestNI8pXz5PJ+wjjb
GIGgsHFQPNir7eGlqymqKue7Zln1oUfIUbZqq+k/492VRz5MlxiwLLl6MVEHu4zh
MfkIW1AgXlI2hSnLh7uoELscWMWmMTxuUGsjIUtF3Xgk/rm+Sv7Ki2xHA8DYOD9S
3Dgj0X/KWrUfjTw6OK+BzasT3Kca+5+nJ3HSmwDKm0+AK/AxzAbseedjyKtkmtOE
1fPcjeRL1CtGwj6mn+Y1tZeljJKuhQCEZVEDrQZC3N5T83ApSv4l/Yx/F/k04PL0
akZeJ1CbGXdtVlali82CKHoK5NGieYkMy9Kbh2zL2wIDAQABoxMwETAPBgNVHRMB
Af8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQAwSVQhg7trxxZcABrp8m4xcCni
288eRvLO7XibqFrPqomus6xA2quk0P47CBqzd9gxLAoaPVrvQy1Hz2H0h9C4PId/
aOroKc5SqynpSYWCdxZ6RqsfJHpoHOE9khsmr2U1yVaKFHwGi7TGmK9srmPx4xFt
7skUli/gpek9oc4lEtWxxmxTMeby/D5XrvMkZRDLYEGzaXwblou6UT3k7Dnf9It/
iaR8PmpJLvZMWwteka4DKLS6ZFkmPm7L2mFDMsqgKCsKRgI51cSaUlLIbqt1l1xP
pGjvrkvR+RYVFLDXRNMRftK61665vMyddmKw2xWxbTFssprp4f2yuxjbBE2M
-----END CERTIFICATE-----
"#);
        }
        env::set_var("PRIVATE_CA_KEY",
r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCZJVvihcZ70ch6
ckqbLtdxIEmUhJ6y00jylfPk8n7CONsYgaCwcVA82Kvt4aWrKaoq57tmWfWhR8hR
tmqr6T/j3ZVHPkyXGLAsuXoxUQe7jOEx+QhbUCBeUjaFKcuHu6gQuxxYxaYxPG5Q
ayMhS0XdeCT+ub5K/sqLbEcDwNg4P1LcOCPRf8patR+NPDo4r4HNqxPcpxr7n6cn
cdKbAMqbT4Ar8DHMBux552PIq2Sa04TV89yN5EvUK0bCPqaf5jW1l6WMkq6FAIRl
UQOtBkLc3lPzcClK/iX9jH8X+TTg8vRqRl4nUJsZd21WVqWLzYIoegrk0aJ5iQzL
0puHbMvbAgMBAAECggEAQSfKVHEa1XIWy7WVdTl0Ep6sf2H/DNDkf8T5e4YKFQLQ
gDgiT/8dpo1+dFokvFIhIljt+2k5nlDmcpFcB+DYPE95I9LnDf/EcHrG+HVjh1E0
PCkZ+5N2+fobVQNHoutdYSTiNgh9IQR3YIJ8cz1Nr6BeiPsocUq+jJvYCMpCk4cA
j6LiipytZSyDm4azDOHaHMekCZmZvNGzXuIkQWZvpm2pW3Tdw1IUCX5wpp4gUfxT
DvBTekhBP97suEOaUp9lI6BInbl18fIeaRqakQC+nOlhfNmVI1FCqYKfoUvKbUCB
xqaC7z2oVNOKtsopm5NGLhBK4NGrR89kX01ORHxOiQKBgQDT9138mGald/fp2NZN
95OOMKiA9AAc+0aBxOt4udrlhyBfVC0gFzBfHAsB4oksk4nLT5kJf1SIEEjDAqQQ
hMgVuPqUHrE/2Dn+vcG8SlV2HyeVVE+hGGf5UKCYdDz8/AhFmjPzVIBz6sdN+45U
yTBb66UK1Fl4d1JEAbnJkfydIwKBgQC49docOnWrYEI3ywcF+tXz6Rbbct9KExCn
FJH0gLktdifi2oYGUwjrAvOa+9OlJx5Xnd+8JfjTt3hROxFNBk0tWZ8nlXX59kRt
gWR8yJGrUcOFfQD5yA0ke7KMUYVvgDmenFwXsDUly89pirEsuvLQXsVykHr9tWoq
kcWou00N6QKBgH5le74sgskZCNSBYQmNIIghq9l5prehfyHS8zdCXK2SLlOqNl50
dXvBlS7Cj1ntgLWj+XYYX6fjTgA7iunuxAFwFLxOsROJNMwbC3PkP6H4YfpCFFnT
2+xnj9xZNCUHhUc79M6dDRwSXFa8MtuMPTITCo+yoMedH4k+HjN8wk5RAoGBALfV
lA2OhTnqmKY/oyFsaI7fU5qWGBzVyi1mopL0BhmLYKV3MNLEYQ7Ehj+6oGd79Ap9
ncyxqRk1N9706IM4CilS9H8xbGsfPG/itW/ZIf+3arAYyIl7LqTeVV5mAEwMlDhz
jIz21DxW0DZEZUjiH0i/iVwPAk98qqLY9C56y2FRAoGBAKpFCGYywAIzIbhR45n0
RY+ru6DG+VCiCl+RnjLx4Hlvvaw9LE3JyhhgwORe+Y5eMSFFamaCx6L/+qjROWIe
quApe6+W+Ota++RRKHdOVw7Czyom1Kw68Vr7AH4z8tSdFxAkJ6L2ULMrSkJm1rdW
z6/dmI43PN2//g+0cGs8BL2v
-----END PRIVATE KEY-----
"#);
        let certs = load_ssl_certs(&server.config)?;
        let tls_acceptor = Arc::new(Some(TlsAcceptor::from(certs)));
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
                                if let Some(acceptor) = tls_acceptor.as_ref() {
                                    match acceptor.accept(stream).await {
                                        Ok(stream) => {
                                            let mut peer_id = None;
                                            if let Some(certs) = stream.get_ref().1.peer_certificates() {
                                                if !certs.is_empty() {
                                                    peer_id = Some(peer_hash(&certs[0].to_vec()));
                                                }
                                            }
                                            let service = service_fn(move |req| {
                                                let peer_id = Arc::new(peer_id);
                                                let server = server.clone();
                                                Self::connection_handler(server, req, address, peer_id)
                                            });
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
                                    let service = service_fn(move |req| {
                                        let server = server.clone();
                                        Self::connection_handler(server, req, address, Arc::new(None))
                                    });
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
        peer_id: Arc<Option<PeerId>>
    ) -> Result<Response<StreamingBody>, Error> {
        request.extensions_mut().insert(address);
        request.extensions_mut().insert(peer_id);
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
    pub fn default_service(self, mut service: Service) -> Self {
        let mut s = self;
        service.shared_state.extend(s.shared_state.clone());
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
    pub fn shared_state<T: Send + Sync + 'static>(self, shared_state: T) -> Self {
        let mut s = self;
        s.shared_state.insert(Arc::new(shared_state));
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
