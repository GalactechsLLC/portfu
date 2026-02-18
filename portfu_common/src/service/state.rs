use crate::error::PortfuError;
use crate::service::request::{FromRequest, Request};
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone)]
pub struct State<T: Send + Sync + 'static>(pub Arc<T>);

impl<T: Send + Sync + 'static> State<T> {
    pub fn inner(&self) -> Arc<T> {
        self.0.clone()
    }
}

impl<T: Send + Sync + 'static> AsRef<T> for State<T> {
    fn as_ref(&self) -> &T {
        self.0.as_ref()
    }
}

impl<T: Send + Sync + 'static> Deref for State<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref()
    }
}

impl<T: Send + Sync + 'static> FromRequest<Request> for State<T> {
    type Error = PortfuError;
    fn try_from<'a>(
        value: &'a mut Request,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            value
                .get::<Arc<T>>()
                .cloned()
                .map(State)
                .ok_or(PortfuError::Parsing(format!(
                    "Failed to find State of type {}",
                    std::any::type_name::<T>()
                )))
        })
    }
}
