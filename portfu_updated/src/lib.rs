pub mod prelude {
    pub use http;
    pub use inventory;
    pub use portfu_common::error::PortfuError;
    pub use portfu_common::router::filter as filters;
    pub use portfu_common::router::path::Path;
    pub use portfu_common::router::path::PathImpl;
    pub use portfu_common::router::path::PathName;
    pub use portfu_common::server::Server;
    pub use portfu_common::server::ServiceRegister;
    pub use portfu_common::server::ServiceRegistration;
    pub use portfu_common::server::ServiceRegistry;
    pub use portfu_common::server::builder::ServerBuilder;
    pub use portfu_common::service::Service;
    pub use portfu_common::service::State;
    pub use portfu_common::service::builder::ServiceBuilder;
    pub use portfu_common::service::request::FromRequest;
    pub use portfu_common::service::request::Request;
    pub use portfu_common::service::response::Response;
    pub use portfu_common::service::traits::Service as ServiceTrait;
    pub use portfu_macros_updated::*;
}
