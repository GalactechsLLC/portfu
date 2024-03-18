mod server;
mod ssl;
mod signal;

use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use futures_util::join;
use http::{Response};
use http_body_util::Full;
use hyper::body::{Bytes};
use log::{info};
use tokio::{spawn};
use portfu_core::service::ServiceRequest;
use portfu_macros::get;
use crate::server::{ServerBuilder, ServerConfig};
use crate::signal::await_termination;

#[tokio::main]
async fn main() -> Result<(), Error>{
    let config = ServerConfig {
        host: "localhost".to_string(),
        port: 8080,
        ssl_config: None,
    };
    let server = ServerBuilder::new(config)
        .register(test_fn)
        .register(test_fn2)
        .build();
    println!("{server:?}");
    let server_run_handle = server.run.clone();
    let signal_handle = spawn(async move {
        let _ = await_termination().await;
        server_run_handle.store(false, Ordering::Relaxed);
    });
    let server_handle = async move {
        server.run().await
    };
    match join!(signal_handle, server_handle) {
        (Ok(_), Ok(_)) => {
            info!("Server Shutting Down");
            Ok(())
        }
        (Ok(_), Err(e)) => {
            Err(Error::new(ErrorKind::Other, format!("Error In Server Thread: {e:?}")))
        }
        (Err(e), Ok(_)) => {
            Err(Error::new(ErrorKind::Other, format!("Failed to Join Signal Thread: {e:?}")))
        }
        (Err(e), Err(e2)) => {
            Err(Error::new(ErrorKind::Other, format!("Failed to Join Server Thread and Signal Thread: {e2:?} \n {e:?}")))
        }
    }
}

#[get("/test/{test2}")]
pub async fn test_fn(
    _address: &SocketAddr,
    _request: &ServiceRequest,
    mut response: Response<Full<Bytes>>,
    test2: String
) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>> {
    *response.body_mut() = Full::new(Bytes::from(test2));
    Ok(response)
}

#[get("/test2/{test2}")]
pub async fn test_fn2(
    _address: &SocketAddr,
    _request: &ServiceRequest,
    mut response: Response<Full<Bytes>>,
    test2: String
) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>> {
    *response.body_mut() = Full::new(Bytes::from(test2));
    Ok(response)
}

