mod server;
mod ssl;
mod signal;
pub extern crate portfu_core as core;
pub extern crate portfu_macros as macros;

pub mod prelude {
    use crate::server;
    pub extern crate async_trait;
    pub extern crate http;
    pub extern crate http_body_util;
    pub extern crate hyper;
    pub type ServiceRequest = ::core::service::ServiceRequest;
    pub type ServiceRegistry = ::core::ServiceRegistry;
    pub type ServiceResponse = ::core::ServiceResponse;
    pub type ServerBuilder = server::ServerBuilder;
    pub type SslConfig = server::SslConfig;
    pub type Path = ::core::Path;
    pub type Body<T> = ::core::Body<T>;
    pub type State<T> = ::core::State<T>;
}