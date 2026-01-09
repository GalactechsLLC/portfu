use crate::services::editor::ServiceEditor;
use crate::services::users::UserManager;
use crate::stores::UserStore;
use portfu::pfcore::npm_service::NpmSinglePageApp;
use portfu::prelude::ServiceGroup;
use portfu::wrappers::sessions::SessionManager;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

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
        let session_manager = Arc::new(SessionManager::default());
        ServiceGroup::default()
            .shared_state(admin.user_datastore)
            .wrap(session_manager.clone())
            .task(session_manager)
            .sub_group(ServiceEditor::default())
            .sub_group(NpmSinglePageApp::new(
                PathBuf::from(env::var("SVELTE_SOURCE").unwrap()),
                PathBuf::from(env::var("SVELTE_OUTPUT").unwrap()),
                "build".to_string(),
            ))
            .sub_group(UserManager::<U>::default())
    }
}
