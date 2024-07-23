use std::env;
use std::path::PathBuf;
use crate::services::editor::ServiceEditor;
use crate::services::users::UserManager;
use crate::stores::UserStore;
use portfu::prelude::ServiceGroup;
use portfu::wrappers::sessions::SessionWrapper;
use std::sync::Arc;
use portfu::pfcore::npm_service::NpmSinglePageApp;

pub mod auth;
pub mod services;
pub mod stores;
pub mod themes;
pub mod users;
pub mod utils;

// #[static_files("front_end_dist/")]
// pub struct StaticFiles;

pub struct PortfuAdmin<T: UserStore> {
    pub user_datastore: T,
}
impl<U: UserStore> From<PortfuAdmin<U>> for ServiceGroup {
    fn from(admin: PortfuAdmin<U>) -> ServiceGroup {
        ServiceGroup::default()
            .shared_state(admin.user_datastore)
            .wrap(Arc::new(SessionWrapper::default()))
            .sub_group(ServiceEditor::default())
            .sub_group(NpmSinglePageApp::new(
                PathBuf::from(env::var("SVELTE_SOURCE").unwrap()),
                PathBuf::from(env::var("SVELTE_OUTPUT").unwrap()),
                "build".to_string()
            ))
            .sub_group(UserManager::<U>::default())
    }
}
