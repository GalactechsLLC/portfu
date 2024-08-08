use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem::swap;

pub struct CircularCache<K: Eq + PartialEq + Hash, V, const N: usize> {
    keys: [Option<K>; N],
    hashes: [Option<u64>; N],
    values: [Option<V>; N],
    index: Option<usize>,
}
impl<K: Eq + PartialEq + Hash, V, const N: usize> Default for CircularCache<K, V, N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<K: Eq + PartialEq + Hash, V, const N: usize> CircularCache<K, V, N> {
    const KEY_REPEAT_VALUE: Option<K> = None;
    const HASH_REPEAT_VALUE: Option<u64> = None;
    const VAL_REPEAT_VALUE: Option<V> = None;

    pub fn new() -> Self {
        Self {
            keys: [Self::KEY_REPEAT_VALUE; N],
            hashes: [Self::HASH_REPEAT_VALUE; N],
            values: [Self::VAL_REPEAT_VALUE; N],
            index: None,
        }
    }
    pub fn keys(&self) -> &[Option<K>] {
        &self.keys
    }
    pub fn values(&self) -> &[Option<V>] {
        &self.values
    }
    pub fn first(&self, key: &K) -> Option<&V> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let search_hash = hasher.finish();
        for (i, k) in self.hashes.iter().enumerate() {
            if *k == Some(search_hash) {
                return self.values[i].as_ref();
            }
        }
        None
    }
    pub fn get(&self, key: &K) -> Vec<&Option<V>> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let search_hash = hasher.finish();
        let mut slices = vec![];
        for (i, k) in self.hashes.iter().enumerate() {
            if *k == Some(search_hash) {
                slices.push(&self.values[i])
            }
        }
        slices
    }
    pub fn get_all(&self, key: &K) -> Vec<&Option<V>> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let search_hash = hasher.finish();
        let mut slices = vec![];
        for (i, k) in self.hashes.iter().enumerate() {
            if *k == Some(search_hash) {
                slices.push(&self.values[i])
            }
        }
        slices
    }
    pub fn contains(&self, key: &K) -> bool {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let search_hash = hasher.finish();
        self.hashes.iter().any(|k| *k == Some(search_hash))
    }
    pub fn insert(&mut self, key: K, value: V) -> (Option<K>, Option<V>) {
        let index = if let Some(index) = &mut self.index {
            *index = index.wrapping_add(1);
            *index
        } else {
            self.index = Some(0);
            0
        };
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let mut hash = Some(hasher.finish());
        let mut key = Some(key);
        let mut value = Some(value);
        swap(&mut self.keys[index % N], &mut key);
        swap(&mut self.hashes[index % N], &mut hash);
        swap(&mut self.values[index % N], &mut value);
        (key, value)
    }
    pub fn replace(&mut self, key: K, value: V) -> Option<V> {
        let key = Some(key);
        for (i, k) in self.keys.iter().enumerate() {
            if *k == key {
                let mut value = Some(value);
                swap(&mut self.values[i], &mut value);
                return value;
            }
        }
        None
    }
    pub fn slice(&self) -> &[Option<V>] {
        match self.index {
            None => &[],
            Some(index) => {
                if index < N {
                    &self.values[0..index]
                } else {
                    &self.values
                }
            }
        }
    }
}
