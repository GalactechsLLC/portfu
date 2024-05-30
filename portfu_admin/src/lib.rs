use portfu::macros::static_files;
use portfu::prelude::ServiceGroup;
use portfu::wrappers::sessions::SessionWrapper;
use std::sync::Arc;
use crate::services::editor::ServiceEditor;
use crate::services::themes::ThemeSelector;
use crate::services::users::UserManager;
use crate::stores::UserStore;

pub mod themes;
pub mod users;
pub mod stores;
pub mod services;
pub mod auth;
pub mod utils;


#[static_files("front_end_dist/")]
pub struct StaticFiles;

pub struct PortfuAdmin<T: UserStore> {
    pub user_datastore: T,
}
impl<U: UserStore> From<PortfuAdmin<U>> for ServiceGroup {
    fn from(admin: PortfuAdmin<U>) -> ServiceGroup {
        ServiceGroup::default()
            .shared_state(admin.user_datastore)
            .wrap(Arc::new(SessionWrapper::default()))
            .sub_group(ServiceEditor::default())
            .sub_group(StaticFiles)
            .sub_group(UserManager::<U>::default())
            .service(ThemeSelector::default())
    }
}