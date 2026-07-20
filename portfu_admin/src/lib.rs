pub mod auth;
pub mod services;
pub mod stores;
pub mod themes;
pub mod users;
pub mod utils;

#[cfg(feature = "admin_ui")]
use crate::services::editor::ServiceEditor;
#[cfg(feature = "admin_ui")]
use crate::services::users::UserManager;
#[cfg(feature = "admin_ui")]
use crate::stores::UserStore;
#[cfg(feature = "admin_ui")]
use portfu::prelude::ServiceGroup;
#[cfg(feature = "admin_ui")]
use portfu::wrappers::sessions::SessionManager;
#[cfg(feature = "admin_ui")]
use std::sync::Arc;

#[cfg(feature = "admin_ui")]
pub struct PortfuAdmin<T: UserStore> {
    pub user_datastore: T,
}
#[cfg(feature = "admin_ui")]
impl<U: UserStore> From<PortfuAdmin<U>> for ServiceGroup {
    fn from(admin: PortfuAdmin<U>) -> ServiceGroup {
        let session_manager = Arc::new(SessionManager::default());
        ServiceGroup::default()
            .shared_state(admin.user_datastore)
            .wrap(session_manager.clone())
            .task(session_manager)
            .sub_group(ServiceEditor::default())
            .sub_group(UserManager::<U>::default())
    }
}
