use std::collections::HashMap;
use std::hash::Hash;
use std::io::Error;
use std::sync::Arc;
use tokio::sync::RwLock;
use portfu::prelude::async_trait::async_trait;
use crate::stores::{DataStore, DataStoreEntry, SearchParams};

#[derive(Default)]
pub struct MemoryDataStore<K: Eq + Hash + Send + Sync, T: DataStoreEntry<K>> {
    data: Arc<RwLock<HashMap<K, T>>>,
}
impl<K: Eq + Hash + Send + Sync, T: DataStoreEntry<K>> MemoryDataStore<K, T> {
    fn required_matches(params: SearchParams) -> Vec<(String, String)> {
        params
            .fields
            .into_iter()
            .filter_map(|(k, v)| {
                if T::parameters().contains(&k.as_str()) {
                    Some((k, v))
                } else {
                    None
                }
            })
            .collect()
    }
}
#[async_trait]
impl<K: Eq + Hash + Send + Sync, T: DataStoreEntry<K>> DataStore<K, T, Error> for MemoryDataStore<K, T> {
    async fn init(&self) -> Result<(), Error> {
        Ok(())
    }

    async fn search(&self, params: SearchParams) -> Result<Vec<T>, Error> {
        let required_matches = Self::required_matches(params);
        Ok(self
            .data
            .read()
            .await
            .iter()
            .filter(|(_, v)| {
                required_matches
                    .iter()
                    .all(|(mk, mv)| v.matches(mk.as_str(), mv.as_str()))
            })
            .map(|(_, v)| v.clone())
            .collect())
    }

    async fn get(&self, key: &K) -> Result<Option<T>, Error> {
        Ok(self.data.read().await.get(key).cloned())
    }


    async fn get_all(&self) -> Result<Vec<T>, Error> {
        Ok(self.data.read().await.values().cloned().collect())
    }

    async fn insert(&self, t: T) -> Result<Option<T>, Error> {
        Ok(self.data.write().await.insert(t.key_value(), t))
    }

    async fn update(&self, t: T) -> Result<Option<T>, Error> {
        Ok(self.data.write().await.insert(t.key_value(), t))
    }

    async fn delete(&self, t: T) -> Result<Option<T>, Error> {
        Ok(self.data.write().await.remove(&t.key_value()))
    }
}
