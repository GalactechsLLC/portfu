use crate::stores::{DataStore, DataStoreEntry, DatabaseEntry, SearchParams};
use portfu::prelude::async_trait::async_trait;
use sqlx::postgres::PgRow;
use sqlx::{Decode, Encode, Executor, FromRow, PgPool, Postgres, Row, Type};
use std::io::{Error, ErrorKind};
use std::marker::PhantomData;

pub struct PostgresDataStore<P: Sync + Send, T: DataStoreEntry<P> + for<'r> FromRow<'r, PgRow>> {
    _phantom_data: PhantomData<(P, T)>,
    connection: PgPool,
}
impl<P: Sync + Send, T: DataStoreEntry<P> + DatabaseEntry<PgRow, P>> PostgresDataStore<P, T> {
    pub fn new(connection: PgPool) -> Self {
        Self {
            _phantom_data: Default::default(),
            connection,
        }
    }
}
#[async_trait]
impl<
        P: Sync + Send + for<'r> Encode<'r, Postgres> + for<'r> Decode<'r, Postgres> + Type<Postgres>,
        T: DataStoreEntry<P> + DatabaseEntry<PgRow, P>,
    > DataStore<P, T, Error> for PostgresDataStore<P, T>
{
    async fn init(&self) -> Result<(), Error> {
        let conn = self.connection.acquire().await.map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to acquire connection: {e:?}"),
            )
        })?;
        T::table_init(conn)
    }

    async fn search(&self, mut params: SearchParams) -> Result<Vec<T>, Error> {
        let mut conn = self.connection.acquire().await.map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to acquire connection: {e:?}"),
            )
        })?;
        T::filter_invalid_params(&mut params);
        let mut query = format!("SELECT * FROM {} ", T::table());
        if !params.fields.is_empty() {
            query.push_str("WHERE ");
            for (index, (field, _)) in params.fields.iter().enumerate() {
                query.extend(format!("{field} LIKE '%' || ${index} || '%' ").chars());
                if index != params.fields.len() - 1 {
                    query.push_str("OR ");
                }
            }
        }
        if params.page > 0 && params.page_size > 0 {
            query.extend(
                format!(
                    "LIMIT {} OFFSET {}",
                    params.page_size,
                    params.page_size * (params.page - 1)
                )
                .chars(),
            );
        } else if params.limit > 0 {
            query.extend(format!("LIMIT {}", params.limit).chars());
        }
        let mut query = sqlx::query(&query);
        if !params.fields.is_empty() {
            for (_, val) in params.fields.iter() {
                query = query.bind(val);
            }
        }
        match conn.fetch_all(query).await {
            Ok(results) => {
                results
                    .into_iter()
                    .try_fold(Vec::new(), |mut v, r| -> Result<Vec<T>, Error> {
                        let t = T::from_row(&r)
                            .map_err(|e| Error::new(ErrorKind::InvalidData, format!("{e:?}")))?;
                        v.push(t);
                        Ok(v)
                    })
            }
            Err(e) => {
                return match e {
                    sqlx::Error::RowNotFound => Ok(vec![]),
                    _ => Err(Error::new(ErrorKind::Other, format!("{e:?}"))),
                };
            }
        }
    }
    async fn get(&self, key: &P) -> Result<Option<T>, Error> {
        let mut conn = self.connection.acquire().await.map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to acquire connection: {e:?}"),
            )
        })?;
        let query = format!("SELECT * FROM {} WHERE {} = $1", T::table(), T::key_name());
        let query = sqlx::query(&query).bind(key);
        match conn.fetch_one(query).await {
            Ok(row) => T::from_row(&row)
                .map(Some)
                .map_err(|e| Error::new(ErrorKind::InvalidData, format!("{e:?}"))),
            Err(e) => {
                return match e {
                    sqlx::Error::RowNotFound => Ok(None),
                    _ => Err(Error::new(ErrorKind::Other, format!("{e:?}"))),
                };
            }
        }
    }

    async fn get_all(&self) -> Result<Vec<T>, Error> {
        let mut conn = self.connection.acquire().await.map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to acquire connection: {e:?}"),
            )
        })?;
        let query = format!("SELECT * FROM {} ", T::table());
        let query = sqlx::query(&query);
        match conn.fetch_all(query).await {
            Ok(results) => {
                results
                    .into_iter()
                    .try_fold(Vec::new(), |mut v, r| -> Result<Vec<T>, Error> {
                        let t = T::from_row(&r)
                            .map_err(|e| Error::new(ErrorKind::InvalidData, format!("{e:?}")))?;
                        v.push(t);
                        Ok(v)
                    })
            }
            Err(e) => {
                return match e {
                    sqlx::Error::RowNotFound => Ok(vec![]),
                    _ => Err(Error::new(ErrorKind::Other, format!("{e:?}"))),
                };
            }
        }
    }

    async fn insert(&self, t: T) -> Result<Option<T>, Error> {
        let mut transaction = self.connection.begin().await.map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to create Transaction: {e:?}"),
            )
        })?;
        let mut query = format!("INSERT INTO {} (", T::table());
        for (index, field) in T::parameters().iter().enumerate() {
            query.push_str(field);
            if index != T::parameters().len() - 1 {
                query.push_str(", ");
            } else {
                query.push(' ');
            }
        }
        query.push_str(") VALUES ( ");
        for (index, _) in T::parameters().iter().enumerate() {
            query.extend(format!("${index}").chars());
            if index != T::parameters().len() - 1 {
                query.push_str(", ");
            } else {
                query.push(' ');
            }
        }
        query.extend(format!(") RETURNING {};", T::key_name()).chars());
        let mut query = sqlx::query(&query);
        for name in T::parameters().iter() {
            query = t.bind(query, name);
        }
        match transaction.fetch_one(query).await {
            Ok(results) => {
                let key: P = results
                    .try_get::<P, usize>(0)
                    .map_err(|e| Error::new(ErrorKind::InvalidData, format!("{e:?}")))?;
                self.get(&key).await
            }
            Err(e) => {
                transaction.rollback().await.map_err(|e| {
                    Error::new(
                        ErrorKind::InvalidData,
                        format!("Failed to Rollback Transaction: {e:?}"),
                    )
                })?;
                Err(Error::new(ErrorKind::Other, format!("{e:?}")))
            }
        }
    }

    async fn update(&self, t: T) -> Result<Option<T>, Error> {
        let mut transaction = self.connection.begin().await.map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to create Transaction: {e:?}"),
            )
        })?;
        let mut query = format!("UPDATE {} SET ", T::table());
        let mut index = 1;
        for field in T::parameters()
            .iter()
            .filter(|name| **name != T::key_name())
        {
            query.extend(format!("{field}=${index}").chars());
            if index != T::parameters().len() - 1 {
                query.push_str(", ");
            } else {
                query.push(' ');
            }
            index += 1;
        }
        query.extend(format!("WHERE {} = ${index}", T::key_name()).chars());
        let mut query = sqlx::query(&query);
        for name in T::parameters()
            .iter()
            .filter(|name| **name != T::key_name())
        {
            query = t.bind(query, name);
        }
        query = t.bind(query, T::key_name());
        match transaction.execute(query).await {
            Ok(rows) => {
                if rows.rows_affected() > 1 {
                    transaction.rollback().await.map_err(|e| {
                        Error::new(
                            ErrorKind::InvalidData,
                            format!("Failed to Rollback Transaction: {e:?}"),
                        )
                    })?;
                    Err(Error::new(ErrorKind::Other, "TOO MANY ROWS AFFECTED"))
                } else {
                    transaction.commit().await.map_err(|e| {
                        Error::new(
                            ErrorKind::InvalidData,
                            format!("Failed to Commit Transaction: {e:?}"),
                        )
                    })?;
                    Ok(Some(t))
                }
            }
            Err(e) => {
                transaction.rollback().await.map_err(|e| {
                    Error::new(
                        ErrorKind::InvalidData,
                        format!("Failed to Rollback Transaction: {e:?}"),
                    )
                })?;
                Err(Error::new(ErrorKind::Other, format!("{e:?}")))
            }
        }
    }

    async fn delete(&self, t: T) -> Result<Option<T>, Error> {
        let mut transaction = self.connection.begin().await.map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to create Transaction: {e:?}"),
            )
        })?;
        let query = format!("DELETE FROM {} WHERE {} = $1", T::table(), T::key_name());
        let mut query = sqlx::query(&query);
        query = t.bind(query, T::key_name());
        match transaction.execute(query).await {
            Ok(rows) => {
                if rows.rows_affected() > 1 {
                    transaction.rollback().await.map_err(|e| {
                        Error::new(
                            ErrorKind::InvalidData,
                            format!("Failed to Rollback Transaction: {e:?}"),
                        )
                    })?;
                    Err(Error::new(ErrorKind::Other, "TOO MANY ROWS AFFECTED"))
                } else {
                    Ok(Some(t))
                }
            }
            Err(e) => {
                transaction.rollback().await.map_err(|e| {
                    Error::new(
                        ErrorKind::InvalidData,
                        format!("Failed to Rollback Transaction: {e:?}"),
                    )
                })?;
                Err(Error::new(ErrorKind::Other, format!("{e:?}")))
            }
        }
    }
}
