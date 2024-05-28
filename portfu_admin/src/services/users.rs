use crate::users::User;
use crate::{UserStore};
use portfu::macros::get;
use portfu::pfcore::ServiceRegister;
use portfu::prelude::http::Extensions;
use portfu::prelude::log::warn;
use portfu::prelude::{ServiceData, ServiceGroup, ServiceRegistry};
use std::io::{Error, ErrorKind};
use std::marker::PhantomData;
use std::sync::Arc;
use crate::stores::DataStore;

#[get("/pf_admin/users")]
pub async fn list_users<D: DataStore<i64, User, Error> + Send + Sync + 'static>(
    data: &mut ServiceData,
) -> Result<Vec<u8>, Error> {
    let user_store: Option<Arc<D>> = data.request.get().cloned();
    if let Some(user_store) = user_store {
        serde_json::to_vec(&user_store.get_all().await?).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to Convert to JSON: {e:?}"),
            )
        })
    } else {
        warn!("Failed to find UserStore");
        Ok(vec![])
    }
}

pub struct UserManager<D: UserStore> {
    service_group: ServiceGroup,
    _phantom_data: PhantomData<D>,
}
impl<D: UserStore> Default for UserManager<D> {
    fn default() -> Self {
        Self {
            service_group: ServiceGroup::default().service(list_users::<D>::default()),
            _phantom_data: Default::default(),
        }
    }
}
impl<D: UserStore> ServiceRegister for UserManager<D> {
    fn register(self, service_registry: &mut ServiceRegistry, shared_state: Extensions) {
        self.service_group.register(service_registry, shared_state);
    }
}
impl<D: UserStore> From<UserManager<D>> for ServiceGroup {
    fn from(value: UserManager<D>) -> Self {
        value.service_group
    }
}
