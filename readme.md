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

Custom services can be created with a struct that implements `Into<Service>`.
When a request is sent to the server it will search for the first registered Service where the below are true:
- The Path string of the service matches the requests URI path
- The Filters attached to the service all return ```FilterResult::Allow```

`ServiceGroup` registers a collection of services in order. Filters and middleware apply only to services added after them; subgroups inherit their parent group's configuration.
`ServiceGroup::shared_state` adds `Arc`-backed state to the server's default scope, and a later registration of the same type replaces an earlier value.

```rust
let api_services = ServiceGroup::new()
    .shared_state(api_state)
    .filter(auth_filter)
    .service(users_service)
    .sub_group(
        ServiceGroup::new()
            .filter(admin_filter)
            .service(admin_service),
    );

let server = ServerBuilder::new()
    .service_group(api_services)
    .build();
```

Websocket handlers run in spawned upgrade tasks. Use owned `RequestHeaders` parameters; Portfu clones the request headers before the task starts. Use `WebSocket` for server-side connections and `ClientWebSocket` for outbound `connect_async` connections.
