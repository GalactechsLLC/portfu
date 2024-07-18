mod tasks;
mod config;
mod kube;

use std::env;
use std::str::FromStr;
use log::{info, LevelFilter};
use simple_logger::SimpleLogger;
use portfu::prelude::*;

const DEFAULT_HOSTNAME: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 8080;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .init()
        .unwrap();
    let host = env::var("HOSTNAME").unwrap_or_else(|_| DEFAULT_HOSTNAME.to_string());
    let port = env::var("PORT").map(|s| u16::from_str(&s).unwrap_or(DEFAULT_PORT)).unwrap_or(DEFAULT_PORT);
    info!("Starting Operator on {host}:{port}");
    let server = ServerBuilder::default()
        .host(host)
        .port(port)
        .build();
    server.run().await
}
