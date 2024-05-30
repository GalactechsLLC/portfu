use std::hash::Hash;
use std::io::Error;
use portfu::prelude::async_trait::async_trait;
use crate::stores::{DataStore, DataStoreEntry, SearchParams};
use crate::utils::cache::{CacheMap};

#[derive(Default)]
pub struct CacheDataStore<K: Eq + Clone + Hash + Send + Sync, T: DataStoreEntry<K>, D: DataStore<K, T, Error> + Send + Sync> {
    _cache: CacheMap<K, T>,
    data_store: D
}
#[async_trait]
impl<K: Eq + Clone + Hash + Send + Sync, T: DataStoreEntry<K>, D: DataStore<K, T, Error> + Send + Sync> DataStore<K, T, Error> for CacheDataStore<K, T, D> {
    async fn init(&self) -> Result<(), Error> {
        self.data_store.init().await
    }

    async fn search(&self, params: SearchParams) -> Result<Vec<T>, Error> {
        self.data_store.search(params).await
    }

    async fn get(&self, key: &K) -> Result<Option<T>, Error> {
        self.data_store.get(key).await
    }


    async fn get_all(&self) -> Result<Vec<T>, Error> {
        self.data_store.get_all().await
    }

    async fn insert(&self, t: T) -> Result<Option<T>, Error> {
        self.data_store.insert(t).await
    }

    async fn update(&self, t: T) -> Result<Option<T>, Error> {
        self.data_store.update(t).await
    }

    async fn delete(&self, t: T) -> Result<Option<T>, Error> {
        self.data_store.delete(t).await
    }
}
