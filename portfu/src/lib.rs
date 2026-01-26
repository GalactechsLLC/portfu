pub mod client;
pub mod filters;
pub mod wrappers;

pub extern crate portfu_core as pfcore;
pub extern crate portfu_macros as macros;

pub mod prelude {
    pub extern crate async_trait;
    pub extern crate futures_util;
    pub extern crate http;
    pub extern crate http_body_util;
    pub extern crate hyper;
    pub extern crate hyper_util;
    pub extern crate log;
    pub extern crate once_cell;
    pub extern crate serde_json;
    pub extern crate tokio_tungstenite;
    pub extern crate uuid;
    pub type Service = ::pfcore::services::Service;
    pub type ServiceType = ::pfcore::ServiceType;
    pub type Server = ::pfcore::server::Server;
    pub type ServerBuilder = ::pfcore::server::builder::ServerBuilder;
    pub type SslConfig = ::pfcore::server::config::SslConfig;
    pub type ServiceResponse = ::pfcore::services::response::ServiceResponse;
    pub type ServiceGroup = ::pfcore::services::group::ServiceGroup;
    pub type IncomingRequest = ::pfcore::services::request::ServiceRequest;
    pub type ServiceRegistry = ::pfcore::ServiceRegistry;
    pub type ServiceData = ::pfcore::ServiceData;
    pub type Path = ::pfcore::Path;
    pub type Body<T> = ::pfcore::Body<T>;
    pub type State<T> = ::pfcore::State<T>;
    pub type WebSocket = ::pfcore::sockets::WebSocket;
    pub type WebsocketMsgStream = ::pfcore::sockets::WebsocketMsgStream;
    pub type WebsocketConnection = ::pfcore::sockets::WebsocketConnection;
    pub type Peers = ::pfcore::sockets::Peers;
}
