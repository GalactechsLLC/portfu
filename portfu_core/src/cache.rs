use std::mem::swap;

pub struct CircularCache<K, V, const N: usize> {
    keys: [Option<K>; N],
    values: [Option<V>; N],
    index: Option<usize>,
}
impl<K: Eq + PartialEq, V, const N: usize> CircularCache<K, V, N> {
    const KEY_REPEAT_VALUE: Option<K> = None;
    const VAL_REPEAT_VALUE: Option<V> = None;

    pub fn new() -> Self {
        Self {
            keys: [Self::KEY_REPEAT_VALUE; N],
            values: [Self::VAL_REPEAT_VALUE; N],
            index: None,
        }
    }
    pub fn first(&self, key: &K) -> Option<&V> {
        for (i, k) in self.keys.iter().enumerate() {
            if k.as_ref() == Some(key) {
                return self.values[i].as_ref();
            }
        }
        None
    }
    pub fn get(&self, key: &K) -> Vec<&Option<V>> {
        let mut slices = vec![];
        for (i, k) in self.keys.iter().enumerate() {
            if k.as_ref() == Some(key) {
                slices.push(&self.values[i])
            }
        }
        slices
    }
    pub fn keys(&self) -> &[Option<K>] {
        &self.keys
    }
    pub fn values(&self) -> &[Option<V>] {
        &self.values
    }
    pub fn get_all(&self, key: &K) -> Vec<&Option<V>> {
        let mut slices = vec![];
        for (i, k) in self.keys.iter().enumerate() {
            if k.as_ref() == Some(key) {
                slices.push(&self.values[i])
            }
        }
        slices
    }
    pub fn contains(&self, key: &K) -> bool {
        self.keys.iter().find(|k| k.as_ref() == Some(key)).is_some()
    }
    pub fn insert(&mut self, key: K, value: V) -> (Option<K>, Option<V>) {
        let index = if let Some(index) = &mut self.index {
            *index = index.wrapping_add(1);
            *index
        } else {
            self.index = Some(0);
            0
        };
        let mut key = Some(key);
        let mut value = Some(value);
        swap(&mut self.keys[index % N], &mut key);
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
