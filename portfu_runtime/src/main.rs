use log::{error, warn, LevelFilter};
use portfu::macros::files;
use portfu::pfcore::service::ServiceGroup;
use portfu::prelude::*;
use portfu_admin::users::User;
use portfu_admin::{MemoryDataStore, PortfuAdmin};
use simple_logger::SimpleLogger;
use std::str::FromStr;

#[files("front_end_dist/")]
pub struct EditableFiles;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    SimpleLogger::new()
        .with_level(LevelFilter::Debug)
        .init()
        .unwrap();
    let host = std::env::var("HOST").unwrap_or_else(|_| {
        warn!("HOST not set, Using 0.0.0.0");
        String::from("0.0.0.0")
    });
    let port = std::env::var("PORT")
        .map(|s| {
            u16::from_str(&s).unwrap_or_else(|e| {
                error!("Invalid PORT {e:?}, Falling back to 8080");
                8080
            })
        })
        .unwrap_or_else(|_| {
            warn!("PORT not set, Using 8080");
            8080
        });
    let server = ServerBuilder::default()
        .register(
            ServiceGroup::default()
                .sub_group(EditableFiles)
                .sub_group(PortfuAdmin::<MemoryDataStore<User>>::default()),
        )
        .host(host)
        .port(port)
        .build();
    server.run().await
}
