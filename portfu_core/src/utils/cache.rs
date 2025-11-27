use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem::{self, swap, MaybeUninit};
use std::ops::Rem;
use std::ptr;

/// A fixed-size, first-in, first-out (FIFO) cache implementation
/// that uses a circular buffer structure.
///
/// Lookups are O(N) because the entire array must be scanned.
/// This is designed for small, high throughput caches
pub struct CircularCache<K: Eq + PartialEq + Hash, V, const N: usize> {
    keys: [MaybeUninit<K>; N],
    hashes: [u64; N],
    values: [MaybeUninit<V>; N],
    index: usize,
    length: usize,
}

impl<K: Eq + PartialEq + Hash, V, const N: usize> Default for CircularCache<K, V, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + PartialEq + Hash, V, const N: usize> CircularCache<K, V, N> {

    pub const fn new() -> Self {
        // Enforce a minimum size for practicality
        assert!(N > 0, "CircularCache size N must be greater than 0");
        Self {
            keys: [const { MaybeUninit::uninit() }; N],
            hashes: [0; N],
            values: [const { MaybeUninit::uninit() }; N],
            index: 0,
            length: 0,
        }
    }

    fn iter_initialized(&self) -> Option<impl Iterator<Item = (usize, &K, u64, &V)>> {
        if self.length == 0 {
            None
        } else {
            Some((0..self.length).map(|i| {
                // Safety: We only access up to the current index, which we know
                // has been initialized by 'insert'.
                unsafe {
                    let k_ref = self.keys[i].assume_init_ref();
                    let h_ref = self.hashes[i];
                    let v_ref = self.values[i].assume_init_ref();
                    (i, k_ref, h_ref, v_ref)
                }
            }))
        }
    }

    pub fn first(&self, key: &K) -> Option<&V> {
        let search_hash = Self::hash(&key);
        for (_, stored_key, stored_hash, stored_value) in self.iter_initialized()? {
            if stored_hash == search_hash && stored_key == key {
                return Some(stored_value);
            }
        }
        None
    }

    pub fn get(&self, key: &K) -> Vec<&V> {
        let search_hash = Self::hash(&key);
        let mut slices = Vec::new();
        if let Some(values) = self.iter_initialized() {
            for (_, stored_key, stored_hash, stored_value) in values {
                if stored_hash == search_hash && stored_key == key {
                    slices.push(stored_value);
                }
            }
        }
        slices
    }

    pub fn contains(&self, key: &K) -> bool {
        let search_hash = Self::hash(&key);
        if let Some(values) = self.iter_initialized() {
            for (_, stored_key, stored_hash, _) in values {
                if stored_hash == search_hash && stored_key == key {
                    return true;
                }
            }
        }
        false
    }

    pub fn insert(&mut self, key: K, value: V) -> (Option<K>, Option<V>) {
        let index = self.index.rem(N);
        self.index = self.index.wrapping_add(1);
        let new_hash = Self::hash(&key);
        let (evicted_k, evicted_v) = if self.length == N {
            // Safety: If at capacity the slot is guaranteed to contain initialized data.
            // We read the old data out to return.
            let mut new_key = MaybeUninit::new(key);
            let mut new_value = MaybeUninit::new(value);
            swap(&mut self.keys[index], &mut new_key);
            swap(&mut self.values[index], &mut new_value);
            unsafe { (Some(new_key.assume_init()), Some(new_value.assume_init())) }
        } else {
            self.keys[index].write(key);
            self.values[index].write(value);
            self.length += 1;
            (None, None)
        };
        self.hashes[index] = new_hash;
        (evicted_k, evicted_v)
    }

    pub fn replace(&mut self, key: &K, value: V) -> Option<V> {
        if self.length == 0 {
            return None;
        }
        for i in 0..self.length {
            // Safety: We only access up to the current index, which we know is initialized.
            let stored_key = unsafe { self.keys[i].assume_init_ref() };

            if stored_key == key {
                // Safety: We replace the initialized value in place. We read the old value out
                // which causes it to be dropped/returned, and write the new value in.
                let mut value = MaybeUninit::new(value);
                swap(&mut self.values[i], &mut value);
                return Some(unsafe { value.assume_init() });
            }
        }
        None
    }

    pub fn slice(&self) -> &[V] {
        // Safety: We only slice over elements that we know have been initialized (up to `length`).
        // We transmute the slice of MaybeUninit<V> to a slice of V.
        unsafe {
            let slice = &self.values[0..self.length];
            mem::transmute::<&[MaybeUninit<V>], &[V]>(slice)
        }
    }
    fn hash(key: &K) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }
}

// CRITICAL: Implement Drop to manually clean up initialized elements when the cache goes out of scope.
// Without this, K and V types that implement Drop (like Vec or String) will leak memory.
impl<K: Eq + PartialEq + Hash, V, const N: usize> Drop for CircularCache<K, V, N> {
    fn drop(&mut self) {
        // Safety: We only drop elements within the initialized range (0..length).
        if self.length == 0 {
            return;
        }
        for i in 0..self.length {
            unsafe {
                ptr::drop_in_place(self.keys[i].as_mut_ptr());
                ptr::drop_in_place(self.values[i].as_mut_ptr());
            }
        }
    }
}
