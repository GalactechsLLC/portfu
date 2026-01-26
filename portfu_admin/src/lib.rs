use crate::services::editor::ServiceEditor;
use crate::services::users::UserManager;
use crate::stores::UserStore;
use portfu::prelude::ServiceGroup;
use portfu::wrappers::sessions::SessionManager;
use std::sync::Arc;

pub mod auth;
pub mod services;
pub mod stores;
pub mod themes;
pub mod users;
pub mod utils;

pub struct PortfuAdmin<T: UserStore> {
    pub user_datastore: T,
}
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
