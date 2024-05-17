use crate::service::Service;
use std::fmt::{Debug, Formatter};

pub trait EditFn {
    type Error;
    fn name(&self) -> &str;
    fn edit(&self, service: Service) -> Result<Service, (Self::Error, Service)>;
}
impl<T> Debug for (dyn EditFn<Error = T> + Send + Sync + 'static) {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}
