use log::{info, LevelFilter};
use portfu::pfcore::service::{ServiceGroup};
use portfu::macros::{files, get, post, task, websocket};
use portfu::prelude::*;
use simple_logger::SimpleLogger;
use std::io::Error;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::select;
use tokio::sync::RwLock;
use portfu::wrappers::sessions::SessionWrapper;

#[files("front_end_dist/")]
pub struct StaticFiles;

#[get("/{test2}")]
pub async fn example_fn(
    get_counter: State<RwLock<AtomicUsize>>,
    test2: Path,
) -> Result<String, Error> {
    let val = get_counter
        .inner()
        .write()
        .await
        .fetch_add(1, Ordering::Relaxed);
    let rtn = format!(
        "Path: {}\nrequest_count: {}",
        test2.inner().as_str(),
        val + 1
    );
    Ok(rtn)
}


#[post("/{test2}")]
pub async fn example_fn2(
    _address: SocketAddr,
    get_counter: State<RwLock<AtomicUsize>>,
    body: Body<u32>,
    test2: Path,
) -> Result<String, Error> {
    let val = get_counter
        .inner()
        .write()
        .await
        .fetch_add(1, Ordering::Relaxed);
    let rtn = format!(
        "Path: {}\nBody: {}\nrequest_count: {}",
        test2.inner().as_str(),
        body.inner(),
        val + 1
    );
    Ok(rtn)
}

#[task("")]
pub async fn example_task(state: State<RwLock<AtomicUsize>>) -> Result<(), Error> {
    loop {
        state.inner().read().await.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[websocket("/ws/{test2}")]
pub async fn example_websocket(websocket: WebSocket) -> Result<(), Error> {
    while let Ok(msg) = websocket.next_message().await {
        match msg {
            Some(v) => {
                websocket.send(v).await?;
            }
            None => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .with_colors(true)
        .init()
        .unwrap_or_default();
    let server = ServerBuilder::default()
        .shared_state(RwLock::new(AtomicUsize::new(0))) //Shared State Data is auto wrapped in an Arc
        .shared_state("This value gets Overridden") //Only one version of a type can exist in the Shared data, to get around this use a wrapper struct/enum
        .shared_state("By this value")
        .register(ServiceGroup::default()
            .sub_group(ServiceGroup::default()
                .wrap(Arc::new(SessionWrapper::default()))
                .service(example_fn)
                .service(example_fn2)
                .service(example_websocket {
                    peers: Default::default(),
                })
            )
            .sub_group(StaticFiles)
        )
        .task(example_task)
        .build();
    info!("{server:#?}");
    server.run().await
}