use log::{error, warn, LevelFilter};
use portfu::macros::files;
use portfu::pfcore::service::ServiceGroup;
use portfu::prelude::*;
use portfu_admin::stores::memory::MemoryDataStore;
use portfu_admin::stores::postgres::PostgresDataStore;
use portfu_admin::users::User;
use portfu_admin::PortfuAdmin;
use simple_logger::SimpleLogger;
use sqlx::postgres::PgPoolOptions;
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
    let mut service_group = ServiceGroup::default().sub_group(EditableFiles);
    match std::env::var("DATABASE_URL").ok() {
        Some(_url) => {
            {
                let pg_pool = PgPoolOptions::new()
                    .max_connections(100)
                    .connect(&_url)
                    .await
                    .unwrap();
                service_group =
                    service_group.sub_group(PortfuAdmin::<PostgresDataStore<i64, User>> {
                        user_datastore: PostgresDataStore::new(pg_pool),
                    });
            }
            #[cfg(not(feature = "postgres"))]
            {
                warn!("Database URL Provided but no Database Feature Enabled");
                service_group =
                    service_group.sub_group(PortfuAdmin::<MemoryDataStore<i64, User>> {
                        user_datastore: MemoryDataStore::<i64, User>::default(),
                    });
            }
        }
        None => {
            service_group = service_group.sub_group(PortfuAdmin::<MemoryDataStore<i64, User>> {
                user_datastore: MemoryDataStore::<i64, User>::default(),
            });
        }
    };
    let server = ServerBuilder::default()
        .register(service_group)
        .host(host)
        .port(port)
        .build();
    server.run().await
}
