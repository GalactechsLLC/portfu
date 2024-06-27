pub mod memory;
#[cfg(feature = "postgres")]
pub mod postgres;
mod cache;

use std::io::Error;
#[cfg(feature = "postgres")]
use sqlx::{FromRow, Row};
#[cfg(feature = "postgres")]
use sqlx::{Postgres};
#[cfg(feature = "postgres")]
use sqlx::database::HasArguments;
#[cfg(feature = "postgres")]
use sqlx::query::Query;
use portfu::prelude::async_trait::async_trait;
use crate::users::User;


pub trait UserStore: DataStore<i64, User, Error> + Send + Sync + 'static {}

impl<T: DataStore<i64, User, Error> + Send + Sync + 'static> UserStore for T {}

pub struct SearchParams {
    pub fields: Vec<(String, String)>,
    pub limit: isize,
    pub page: isize,
    pub page_size: isize,
    pub order_by: Option<String>
}

pub trait DataStoreEntry<T>: Default + Send + Sync + Eq + Clone + 'static {
    fn key_name() -> &'static str;
    fn key_value(&self) -> T;
    fn parameters() -> &'static [&'static str];
    fn matches(&self, name: &str, other: &str) -> bool;
    fn filter_invalid_params(params: &mut SearchParams) {
        params
            .fields
            .retain(|(k, _)| {
                Self::parameters().contains(&k.as_str())
            });
    }
}



#[cfg(feature = "postgres")]
pub trait DatabaseEntry<R: Row, P>: for<'r>  FromRow<'r, R> {
    fn bind<'q>(&'q self, query: Query<'q, Postgres, <Postgres as HasArguments>::Arguments>, field: &str)
        -> Query<'q, Postgres, <Postgres as HasArguments>::Arguments>;
    fn database() -> String;
    fn table() -> String;
}

#[async_trait]
pub trait DataStore<K, T: DataStoreEntry<K>, E> {
    async fn init(&self) -> Result<(), E>;
    async fn search(&self, params: SearchParams) -> Result<Vec<T>, E>;
    async fn get(&self, key: &K) -> Result<Option<T>, E>;
    async fn get_all(&self) -> Result<Vec<T>, E>;
    async fn insert(&self, t: T) -> Result<Option<T>, E>;
    async fn update(&self, t: T) -> Result<Option<T>, E>;
    async fn delete(&self, t: T) -> Result<Option<T>, E>;
}
