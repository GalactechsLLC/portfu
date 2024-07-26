mod config;

use crate::config::{Config, DatabaseType};
use log::{info, LevelFilter};
use portfu::pfcore::files::DynamicFiles;
use portfu::pfcore::service::ServiceGroup;
use portfu::prelude::*;
use portfu_admin::services::themes::ThemeSelector;
use portfu_admin::stores::memory::MemoryDataStore;
use portfu_admin::stores::postgres::PostgresDataStore;
use portfu_admin::users::User;
use portfu_admin::PortfuAdmin;
use simple_logger::SimpleLogger;
use sqlx::postgres::PgPoolOptions;
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    SimpleLogger::new()
        .with_level(LevelFilter::Debug)
        .init()
        .unwrap();
    let config = env::var("CONFIG_PATH").map_or(Config::from_env(), |s| {
        Config::try_from(std::path::Path::new(&s)).unwrap()
    });
    let test_res: Result<usize, std::io::Error> = Ok(1024usize);
    info!("{test_res:?}");
    let mut service_group = ServiceGroup::default();
    for directory in config.directories {
        service_group = service_group.sub_group(DynamicFiles {
            root_directory: PathBuf::from(directory),
            editable: true,
        });
    }
    if let Some(db_url) = config.database_url {
        match config.database_type {
            Some(d_type) => match d_type {
                DatabaseType::Mysql => {
                    todo!()
                }
                DatabaseType::Postgres => {
                    let pg_pool = PgPoolOptions::new()
                        .max_connections(100)
                        .connect(&db_url)
                        .await
                        .unwrap();
                    service_group =
                        service_group.sub_group(PortfuAdmin::<PostgresDataStore<i64, User>> {
                            user_datastore: PostgresDataStore::new(pg_pool),
                        });
                }
            },
            None => {
                service_group =
                    service_group.sub_group(PortfuAdmin::<MemoryDataStore<i64, User>> {
                        user_datastore: MemoryDataStore::<i64, User>::default(),
                    });
            }
        };
    } else {
        service_group = service_group.sub_group(PortfuAdmin::<MemoryDataStore<i64, User>> {
            user_datastore: MemoryDataStore::<i64, User>::default(),
        });
    }
    let server = ServerBuilder::default()
        .register(service_group)
        .default_service(ThemeSelector::default().into())
        .host(config.hostname)
        .port(config.port)
        .build();
    server.run().await
}
