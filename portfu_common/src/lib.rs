#[cfg(feature = "oauth")]
pub mod auth;
pub mod error;
pub mod router;
pub mod server;
pub mod service;
pub mod signal;
mod stream;
#[cfg(feature = "websocket")]
pub mod websocket;
pub mod wrappers;

#[cfg(test)]
#[path = "../tests/unit/mod.rs"]
mod unit_tests;
