mod config;
mod kube;
mod tasks;

use ::kube::Client;
use log::{info, LevelFilter};
use portfu::prelude::*;
use portfu_operator_lib::services::kube::KubeNamespace;
use portfu_operator_lib::services::register_services;
use simple_logger::SimpleLogger;
use std::env;
use std::io::Error;
use std::str::FromStr;

const DEFAULT_HOSTNAME: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 8080;

#[tokio::main]
async fn main() -> Result<(), Error> {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .init()
        .unwrap();
    let host = env::var("HOSTNAME").unwrap_or_else(|_| DEFAULT_HOSTNAME.to_string());
    let port = env::var("PORT")
        .map(|s| u16::from_str(&s).unwrap_or(DEFAULT_PORT))
        .unwrap_or(DEFAULT_PORT);
    let namespace = env::var("NAMESPACE").unwrap_or(kube::DEFAULT_NAMESPACE.to_string());
    let kube_client = Client::try_default()
        .await
        .map_err(|e| Error::other(format!("{e:?}")))?;
    info!("Starting Operator on {host}:{port}");
    let server = register_services(
        ServerBuilder::default()
            .shared_state(kube_client)
            .shared_state(KubeNamespace(namespace)),
    )
    .host(host)
    .port(port)
    .build();
    server.run().await
}
