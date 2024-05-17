use log::{info, LevelFilter};
use portfu::endpoints::edit::EditHandler;
use portfu::filters::method::*;
use portfu::filters::{any, has_header};
use portfu::macros::{get, interval, post, static_files, task, websocket};
use portfu::pfcore::service::{IncomingRequest, ServiceGroup};
use portfu::prelude::http::{HeaderName, Response};
use portfu::prelude::*;
use portfu::wrappers::sessions::SessionWrapper;
use simple_logger::SimpleLogger;
use std::io::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::select;
use tokio::sync::RwLock;

#[static_files("front_end_dist/")]
pub struct StaticFiles;

#[get("/")]
pub async fn index(_index: &IncomingRequest) -> Result<Vec<u8>, Error> {
    Ok(STATIC_FILE_index_html.to_vec())
}

#[get("/echo/{path_variable}")]
pub async fn example_get(path_variable: Path) -> Result<String, Error> {
    Ok(path_variable.inner())
}

#[post("/counter")]
pub async fn example_post(get_counter: State<AtomicUsize>) -> Result<String, Error> {
    let val = get_counter.inner().fetch_add(1, Ordering::Relaxed) + 1;
    Ok(val.to_string())
}

#[interval(500u64)]
pub async fn example_interval(state: State<AtomicUsize>) -> Result<(), Error> {
    state.inner().fetch_add(1, Ordering::Relaxed);
    info!("Tick");
    Ok(())
}

#[task]
pub async fn example_task() -> Result<(), Error> {
    info!("Starting Server Task");
    tokio::time::sleep(Duration::from_secs(5)).await;
    info!("Server Task Complete");
    Ok(())
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
        .with_level(LevelFilter::Debug)
        .init()
        .unwrap(); //Init your logger of choice
    let server = ServerBuilder::default() //Start building the Server
        .shared_state(RwLock::new(AtomicUsize::new(0))) //Shared State Data is auto wrapped in an Arc
        .shared_state("This value gets Overridden") //Only one version of a type can exist in the Shared data, to get around this use a wrapper struct/enum
        .shared_state("By this value")
        //Filters applied at the server level apply to all services regardless of when they were registered
        .filter(any(
            "Method Filters".to_string(),
            &[GET.clone(), POST.clone(), PUT.clone(), DELETE.clone()],
        ))
        .register(EditHandler {})
        .register(StaticFiles) //Register Each Service directly with the server
        .register(
            //Sub Groups are also services
            ServiceGroup::default() //Services can be grouped into ServiceGroups to make it easier to apply shared wrappers or filters.
                //Filters at the ServiceGroup level apply to service defined below them only, this is the same with any wrappers
                .service(example_get) //This service is defined above the filter and will not have the filter applied
                .filter(has_header(HeaderName::from_static("content-length")))
                .service(example_post) //This service is defined below the filter and will have the filter applied
                .wrap(Arc::new(SessionWrapper::default())) //The session wrapper will create a session using cookies for each connection
                //All Requests below this will only work for connections that have a session and send the cookie with requests
                .sub_group(
                    //Add another group to this group
                    ServiceGroup::default().service(example_websocket {
                        //Peers Need to be defined for a websocket, to share peers pass the same map to multiple websockets
                        peers: Default::default(),
                    }),
                ),
        )
        .task(example_task) //Add a background task to start when the server is started
        .task(example_interval) //Intervals are also tasks
        .build();
    info!("{server:#?}"); //Servers impl debug so you can see the structure
    server.run().await //Run the server and wait for a termination signal
}
