pub mod endpoints;
pub mod filters;
mod server;
pub mod signal;
mod ssl;
pub mod wrappers;

pub extern crate portfu_core as pfcore;
pub extern crate portfu_macros as macros;

pub mod prelude {
    use crate::server;
    pub extern crate async_trait;
    pub extern crate http;
    pub extern crate http_body_util;
    pub extern crate hyper;
    pub extern crate hyper_util;
    pub extern crate log;
    pub extern crate once_cell;
    pub extern crate tokio_tungstenite;
    pub extern crate uuid;
    pub type Service = ::pfcore::service::Service;
    pub type ServiceGroup = ::pfcore::service::ServiceGroup;
    pub type ServiceRegistry = ::pfcore::ServiceRegistry;
    pub type ServiceData = ::pfcore::ServiceData;
    pub type ServerBuilder = server::ServerBuilder;
    pub type SslConfig = server::SslConfig;
    pub type Path = ::pfcore::Path;
    pub type Body<T> = ::pfcore::Body<T>;
    pub type State<T> = ::pfcore::State<T>;
    pub type WebSocket = ::pfcore::sockets::WebSocket;
    pub type WebsocketConnection = ::pfcore::sockets::WebsocketConnection;
    pub type WebsocketMsgStream = tokio_tungstenite::WebSocketStream<
        hyper_util::rt::tokio::TokioIo<hyper::upgrade::Upgraded>,
    >;
    pub type Peers = ::pfcore::sockets::Peers;
}
