#[cfg(feature = "client")]
pub mod client;

pub mod prelude {
    #[cfg(feature = "client")]
    pub use crate::client;
    #[cfg(feature = "client")]
    pub use crate::client::SupportedBody;
    pub use http;
    pub use http_body_util;
    pub use hyper;
    pub use hyper_util;
    pub use inventory;
    pub use log;
    #[cfg(feature = "maud")]
    pub use maud;
    #[cfg(feature = "maud")]
    pub use maud::{Markup, PreEscaped, Render, html};
    #[cfg(feature = "oauth")]
    pub use portfu_common::auth;
    #[cfg(feature = "oauth")]
    pub use portfu_common::auth::oauth::{
        OAUTH, OAuthIdentity, OAuthToken, SessionOAuthIdentity, SessionOAuthToken,
    };
    pub use portfu_common::error::PortfuError;
    pub use portfu_common::router::filter as filters;
    pub use portfu_common::router::path::Path;
    pub use portfu_common::router::path::PathImpl;
    pub use portfu_common::router::path::PathName;
    pub use portfu_common::server::Server;
    pub use portfu_common::server::ServiceRegister;
    pub use portfu_common::server::ServiceRegistration;
    pub use portfu_common::server::ServiceRegistry;
    pub use portfu_common::server::TaskRegistration;
    pub use portfu_common::server::builder::ServerBuilder;
    pub use portfu_common::server::config::SslConfig;
    pub use portfu_common::service::RequestHeaders;
    pub use portfu_common::service::ResponseHeaders;
    pub use portfu_common::service::Service;
    pub use portfu_common::service::State;
    pub use portfu_common::service::builder::ServiceBuilder;
    pub use portfu_common::service::request::Body;
    pub use portfu_common::service::request::FromRequest;
    pub use portfu_common::service::request::Json;
    pub use portfu_common::service::request::Query;
    pub use portfu_common::service::request::Request;
    pub use portfu_common::service::request::RequestType;
    pub use portfu_common::service::response::Response;
    pub use portfu_common::service::traits::Service as ServiceTrait;
    #[cfg(feature = "websocket")]
    pub use portfu_common::websocket::Message;
    #[cfg(feature = "websocket")]
    pub use portfu_common::websocket::Peers;
    #[cfg(feature = "websocket")]
    pub use portfu_common::websocket::WebSocket;
    #[cfg(feature = "websocket")]
    pub use portfu_common::websocket::WebSocketClient;
    #[cfg(feature = "websocket")]
    pub use portfu_common::websocket::WebsocketConnection;
    pub use portfu_common::wrappers;
    #[cfg(feature = "sessions")]
    pub use portfu_common::wrappers::sessions::{Session, SessionState};
    #[cfg(any(
        feature = "client",
        feature = "endpoint",
        feature = "files",
        feature = "maud",
        feature = "tasks",
        feature = "websocket"
    ))]
    pub use portfu_macros::*;
    #[cfg(feature = "websocket")]
    pub use tokio_tungstenite;
}
