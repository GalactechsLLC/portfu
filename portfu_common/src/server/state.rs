use std::sync::Arc;

pub struct SharedState<T> {
    inner: Arc<T>,
}
impl<T> From<SharedState<T>> for Arc<T> {
    fn from(value: SharedState<T>) -> Self {
        value.inner
    }
}
impl<T> From<T> for SharedState<T> {
    fn from(value: T) -> Self {
        SharedState {
            inner: Arc::new(value),
        }
    }
}
impl<T> From<Arc<T>> for SharedState<T> {
    fn from(inner: Arc<T>) -> Self {
        SharedState { inner }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/server_state.rs"]
mod tests;
