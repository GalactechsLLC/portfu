[![CI](https://github.com/GalactechsLLC/dg_fast_farmer/actions/workflows/ci.yml/badge.svg)](https://github.com/GalactechsLLC/dg_fast_farmer/actions/workflows/ci.yml)

PortFu
=====

An HTTP Library built to simplify Web App Development. 
- Macros for All standard HTTP Methods GET, POST, DELETE ect...
- Data Extractors to easily access and process request data
- Websocket Macro
- Background Task and Interval Macros

Macro Examples
--------
GET Request using a Path Variable
```rust
#[get("/echo/{path_variable}")]
pub async fn example_fn(
    path_variable: Path,
) -> Result<String, Error> {
    Ok(path_variable.inner())
}
```
StaticFiles from a path (Built into the binary at compile time)
```rust
#[static_files("relative/path/to/files/")]
pub struct StaticFiles;
//By default, / is not mapped to index.html, to fix this add the below
//to use a file other than index.html take the path and apply the below function
//path.replace(['/','.',')','(','-',' ','+'], "_").replace("__", "_");
//ie. relative/path/to/files/some_sub_dir/index.json becomes STATIC_FILE_some_sub_dir_index_json
#[get("/")]
pub async fn index() -> Result<Vec<u8>, Error>{
    Ok(STATIC_FILE_index_html.to_vec())
}
```
POST Request with Shared State
```rust
#[post("/counter")]
pub async fn example_fn(
    get_counter: State<AtomicUsize>,
    path_variable: Path,
) -> Result<String, Error> {
    let val = get_counter
        .inner()
        .fetch_add(1, Ordering::Relaxed) + 1;
    Ok(val.to_string())
}
```
Websockets are bound to a path but can share peers if both Websockets
are created with the same peers object, see main function below
```rust
#[websocket("/echo_websocket")]
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
```
Interval running in the background
```rust
#[interval(500u64)] //Will run every 500ms
pub async fn example_interval(state: State<AtomicUsize>) -> Result<(), Error> {
    state.inner().fetch_add(1, Ordering::Relaxed);
    info!("Tick");
    Ok(())
}
```
Task that will run when server is started
```rust
#[task("")]
pub async fn example_task(state: State<AtomicUsize>) -> Result<(), Error> {
    loop {
        state.inner().fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
```

Custom services can be created with a struct that implements ```ServiceRegister + Into<Service>```
When a request is sent to the server it will search for the first registered Service where the below are true:
- The Path string of the service matches the requests URI path
- The Filters attached to the service all return ```FilterResult::Allow```

ServiceGroups can even have sub_groups to have even finer control over services

Here is the main function that would be used for all the services above, including some example filters and wrappers.
```rust
#[tokio::main]
async fn main() -> Result<(), Error> {
    SimpleLogger::default(); //Init your logger of choice
    let server = ServerBuilder::default() //Start building the Server
        .shared_state(RwLock::new(AtomicUsize::new(0))) //Shared State Data is auto wrapped in an Arc
        .shared_state("This value gets Overridden") //Only one version of a type can exist in the Shared data, to get around this use a wrapper struct/enum
        .shared_state("By this value")
        //Filters applied at the server level apply to all services regardless of when they were registered
        .filter(any("Method Filters".to_string(), &[GET.clone(), POST.clone(), PUT.clone(), DELETE.clone()]))
        .register(StaticFiles) //Register Each Service directly with the server
        .register( //Sub Groups are also services
            ServiceGroup::default() //Start the Subgroup
               //Filters at the ServiceGroup level apply to service defined below them only, this is the same with any wrappers
               .service(example_get) //This service is defined above the filter and will not have the filter applied
               .filter(has_header(HeaderName::from_static("content-length")))
               .service(example_post)//This service is defined below the filter and will have the filter applied
               .wrap(Arc::new(SessionWrapper::default())) //The session wrapper will create a session using cookies for each connection
               //All Requests below this will only work for connections that have a session and send the cookie with requests
               .sub_group( //Add another group to this group
                   ServiceGroup::default()
                       .service(example_websocket { //Peers Need to be defined for a websocket, to share peers pass the same map to multiple websockets
                           peers: Default::default(),
                       })
               ),
        )
        .task(example_task) //Add a background task to start when the server is started
        .task(example_interval) //Intervals are also tasks
        .build();
    info!("{server:#?}"); //Servers impl debug so you can see the structure
    server.run().await //Run the server and wait for a termination signal
}
```

