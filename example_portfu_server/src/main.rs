use std::io::{Error};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use portfu::core::filters::GET;
use portfu::core::service::ServiceBuilder;
use portfu::core::ServiceHandler;
use portfu::prelude::*;
use http::Response;
use http_body_util::Full;
use hyper::body::Bytes;
use tokio::sync::RwLock;
use portfu::macros::{files, get, post};

//Here is how to manually Implement a Service
pub struct IndexService{}
#[async_trait::async_trait]
impl ServiceHandler for IndexService {
    fn name(&self) -> &str {
        "index"
    }
    async fn handle(&self, request: ServiceRequest, response: Response<Full<Bytes>>) -> Result<ServiceResponse, ServiceResponse> {
        let mut response = response;
        *response.body_mut() = Full::new(Bytes::from_static("Hello World".as_bytes()));
        Ok(ServiceResponse{
            request,
            response
        })
    }
}

//A simpler way to define a Service is using the appropriate Macro
//The macro name is the Method type of the request, get/post/put/ect...
//The string path is what the service will match against and can contain Variables that are accessed with the Path type as seen below.
//The name of the variable in the function must match the variable in the path defined by the macro. In the example below you see the variable is "test2" in both places
//Data types available for use with default wrappers enabled are "&SockerAddr, Body<T>, and Path, &ServiceState<T>, &AppState<T>". Wrappers can add custom types,
//Only One body should love for each request, Additional Attempts to Load the body will result in blank data
//Depending on what wrappers are enabled there may be more Types available.

#[get("/test/{test2}")]
pub async fn example_fn( //Dynamic Variables calculated at compile time
    test2: Path,
) -> Result<String, Error> {
    Ok(test2.inner())
}

#[post("/test/{test2}")]
pub async fn example_fn2(
    _address: &SocketAddr,
    str_state: State<&'static str>,
    get_counter: State<RwLock<AtomicUsize>>,
    body: Body<u32>,
    test2: Path,
) -> Result<String, Error> {
    let val = get_counter.inner().write().await.fetch_add(1, Ordering::Relaxed);
    let rtn = format!("Path: {}\nBody: {}\nstr_state: {}\nrequest_count: {}", test2.inner().as_str(), body.data, str_state.inner().as_ref(),  val+1);
    Ok(rtn)
}

#[files("front_end_dist/")]
pub struct StaticFiles;

#[tokio::main]
async fn main() -> Result<(), Error>{
   let manual_service = ServiceBuilder::new("/")
        .name("index")
        .filter(GET.clone())
        .handler(Arc::new(IndexService{}))
        .build();
    let server = ServerBuilder::new()
        .shared_state(RwLock::new(AtomicUsize::new(0)))//Shared State Data is auto wrapped in an Arc
        .shared_state("This value gets Overridden")//Only one version of a type cn exist in the Shared data, to get around this use a wrapper struct/enum
        .shared_state("By this value")
        .register(StaticFiles{})
        .register(manual_service)
        .register(example_fn)
        .register(example_fn2)
        .build();
    server.run().await
}