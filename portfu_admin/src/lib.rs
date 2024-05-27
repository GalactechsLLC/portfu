use crate::editor::ServiceEditor;
use crate::theme::ThemeSelector;
use crate::users::manager::UserManager;
use crate::users::User;
use portfu::macros::static_files;
use portfu::prelude::async_trait::async_trait;
use portfu::prelude::ServiceGroup;
use portfu::wrappers::sessions::SessionWrapper;
use std::collections::HashMap;
use std::io::Error;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod editor;
pub mod pages;
pub mod templates;
mod theme;
pub mod users;

#[static_files("front_end_dist/")]
pub struct StaticFiles;

pub trait UserStore: DataStore<User, Error> + Send + Sync + Default + 'static {}

impl<T: DataStore<User, Error> + Send + Sync + Default + 'static> UserStore for T {}

pub struct PortfuAdmin<T: UserStore> {
    pub user_datastore: T,
}
impl<U: UserStore> Default for PortfuAdmin<U> {
    fn default() -> Self {
        Self {
            user_datastore: U::default(),
        }
    }
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

pub struct SearchParams {
    pub fields: Vec<(String, String)>,
    pub limit: isize,
    pub page: isize,
    pub page_size: isize,
}

pub trait DataStoreEntry: Default + Send + Sync + Clone + 'static {
    fn key(&self) -> String;
    fn parameters() -> &'static [&'static str];
    fn matches(&self, name: &str, other: &str) -> bool;
}

#[async_trait]
pub trait DataStore<T: DataStoreEntry, E> {
    async fn init(&self) -> Result<(), E>;
    async fn get_first(&self, params: SearchParams) -> Result<Option<T>, E>;
    async fn get_selected(&self, params: SearchParams) -> Result<Vec<T>, E>;
    async fn get_all(&self) -> Result<Vec<T>, E>;
    async fn insert(&self, t: T) -> Result<Option<T>, E>;
    async fn update(&self, t: T) -> Result<Option<T>, E>;
    async fn delete(&self, t: T) -> Result<Option<T>, E>;
}

#[derive(Default)]
pub struct MemoryDataStore<T: DataStoreEntry> {
    data: Arc<RwLock<HashMap<String, T>>>,
}
impl<T: DataStoreEntry> MemoryDataStore<T> {
    fn required_matches(params: SearchParams) -> Vec<(String, String)> {
        params
            .fields
            .into_iter()
            .filter_map(|(k, v)| {
                if T::parameters().contains(&k.as_str()) {
                    Some((k, v))
                } else {
                    None
                }
            })
            .collect()
    }
}
#[async_trait]
impl<T: DataStoreEntry> DataStore<T, Error> for MemoryDataStore<T> {
    async fn init(&self) -> Result<(), Error> {
        Ok(())
    }
    async fn get_first(&self, params: SearchParams) -> Result<Option<T>, Error> {
        let required_matches = Self::required_matches(params);
        Ok(self
            .data
            .read()
            .await
            .iter()
            .find(|(_, v)| {
                required_matches
                    .iter()
                    .all(|(mk, mv)| v.matches(mk.as_str(), mv.as_str()))
            })
            .map(|(_, v)| v.clone()))
    }

    async fn get_selected(&self, params: SearchParams) -> Result<Vec<T>, Error> {
        let required_matches = Self::required_matches(params);
        Ok(self
            .data
            .read()
            .await
            .iter()
            .filter(|(_, v)| {
                required_matches
                    .iter()
                    .all(|(mk, mv)| v.matches(mk.as_str(), mv.as_str()))
            })
            .map(|(_, v)| v.clone())
            .collect())
    }

    async fn get_all(&self) -> Result<Vec<T>, Error> {
        Ok(self.data.read().await.values().cloned().collect())
    }

    async fn insert(&self, t: T) -> Result<Option<T>, Error> {
        Ok(self.data.write().await.insert(t.key(), t))
    }

    async fn update(&self, t: T) -> Result<Option<T>, Error> {
        Ok(self.data.write().await.insert(t.key(), t))
    }

    async fn delete(&self, t: T) -> Result<Option<T>, Error> {
        Ok(self.data.write().await.remove(&t.key()))
    }
}
