use crate::files::loader::{get_mime_type, read_directory, FileLoader};
use crate::services::builder::ServiceBuilder;
use crate::services::group::ServiceGroup;
use std::collections::HashMap;
use std::io::Error;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct DynamicFiles {
    pub root_directory: PathBuf,
    pub editable: bool,
    pub cache_size_limit: u64,
}
impl TryFrom<DynamicFiles> for ServiceGroup {
    type Error = Error;
    fn try_from(slf: DynamicFiles) -> Result<ServiceGroup, Error> {
        let mut files = HashMap::new();
        log::info!("Canonicalizing Directory: {:?}", slf.root_directory);
        let root_directory = slf.root_directory.canonicalize()?;
        log::info!("Searching for files at: {:?}", root_directory);
        if !root_directory.exists() {
            if let Err(e) = std::fs::create_dir(&root_directory) {
                log::error!("Error Creating Root Directory: {e:?}");
            }
        }
        if let Err(e) = read_directory(&root_directory, root_directory.clone(), &mut files) {
            log::error!("Error Loading files: {e:?}");
        }
        Ok(ServiceGroup {
            filters: vec![],
            wrappers: vec![],
            tasks: vec![],
            services: files
                .into_iter()
                .map(|(name, path)| {
                    let mime = get_mime_type(&path);
                    ServiceBuilder::new(&name)
                        .name(&name)
                        .handler(Arc::new(FileLoader {
                            name,
                            mime,
                            path,
                            editable: slf.editable,
                            cache_threshold: slf.cache_size_limit,
                            cache_status: AtomicBool::default(),
                            cached_value: Arc::new(RwLock::new(Vec::with_capacity(0))),
                        }))
                        .build()
                })
                .collect(),
            shared_state: Default::default(),
        })
    }
}
