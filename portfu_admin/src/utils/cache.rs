use dashmap::DashMap;
use std::collections::VecDeque;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Default, Clone, Debug)]
pub enum CacheEntry<T> {
    Valid(Arc<T>),
    Expired(Arc<T>),
    #[default]
    Empty,
}

#[derive(Debug)]
pub struct CacheItem<T> {
    value: T,
    stale_timeout: Option<Duration>,
    lifetime: Option<Instant>,
    last_access: Instant,
}
impl<T> CacheItem<T> {
    pub fn new(value: T, stale_timeout: Option<Duration>, lifetime: Option<Duration>) -> Self {
        Self {
            value,
            stale_timeout,
            lifetime: lifetime.map(|d| Instant::now() + d),
            last_access: Instant::now(),
        }
    }
    pub fn get(&self) -> &T {
        &self.value
    }
    pub fn is_expired(&self) -> bool {
        if let Some(stale_timeout) = self.stale_timeout {
            Instant::now() - stale_timeout > self.last_access
        } else if let Some(lifetime) = self.lifetime {
            Instant::now() >= lifetime
        } else {
            false
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct CacheMap<K: Hash + PartialEq + Eq + Clone, T> {
    depth: usize,
    stale_timeout: Option<Duration>,
    lifetime: Option<Duration>,
    recently_used: Arc<RwLock<VecDeque<K>>>,
    entries: DashMap<K, Arc<CacheItem<T>>>,
}
impl<K: Hash + PartialEq + Eq + Clone, T> CacheMap<K, T> {
    pub fn new(depth: usize, stale_timeout: Option<Duration>, lifetime: Option<Duration>) -> Self {
        Self {
            depth,
            stale_timeout,
            lifetime,
            recently_used: Default::default(),
            entries: Default::default(),
        }
    }

    pub async fn add(&self, key: K, value: T) {
        self.update_recent(&key).await;
        self.entries.insert(
            key,
            Arc::new(CacheItem::new(value, self.stale_timeout, self.lifetime)),
        );
        self.trim_recent().await;
    }

    async fn update_recent(&self, key: &K) {
        let mut locked = self.recently_used.write().await;
        locked.retain(|k| k != key);
        locked.push_front(key.clone());
    }

    async fn trim_recent(&self) {
        let recent_len = self.recently_used.read().await.len();
        if recent_len > self.depth {
            for r in self
                .recently_used
                .write()
                .await
                .drain(self.depth..)
                .collect::<Vec<K>>()
            {
                self.entries.remove(&r);
            }
        }
    }

    pub async fn get(&self, key: &K) -> CacheEntry<CacheItem<T>> {
        if let Some(cache_item) = self.entries.get(key).map(|e| e.clone()) {
            self.update_recent(key).await;
            if cache_item.is_expired() {
                CacheEntry::Expired(cache_item)
            } else {
                CacheEntry::Valid(cache_item)
            }
        } else {
            CacheEntry::Empty
        }
    }
}
