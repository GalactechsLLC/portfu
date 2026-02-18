use log::LevelFilter;
pub use portfu_updated::prelude::*;
use simple_logger::SimpleLogger;

#[tokio::main(flavor = "multi_thread")]
pub async fn main() -> Result<(), PortfuError> {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .env()
        .init()
        .unwrap();
    ServerBuilder::from_env()
        .global_state(String::from("Test State"))
        .build()
        .run()
        .await
}

#[get("/echo/{path_variable}/admin")]
pub async fn example_fn_sub(
    path_variable: Path,
    state: State<String>,
    request: &mut Request,
) -> Result<String, PortfuError> {
    let uri = request.uri().to_string();
    Ok(format!(
        "URI: {uri}, Path Arg: {}, State: {}",
        path_variable.inner(),
        state.0.as_str()
    ))
}
