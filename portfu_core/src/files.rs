use crate::editable::EditResult;
use crate::service::{BodyType, ServiceBuilder, ServiceGroup};
use crate::{IntoStreamBody, ServiceData, ServiceHandler, ServiceType, StreamingBody};
use futures_util::TryStreamExt;
use http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use http::{HeaderValue, StatusCode};
use http_body::Frame;
use http_body_util::{BodyStream, StreamBody};
use hyper::body::Bytes;
use mime_guess::from_path;
use std::collections::HashMap;
use std::io::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tokio_util::codec::BytesCodec;

pub struct DynamicFiles {
    pub root_directory: PathBuf,
    pub editable: bool,
}
impl From<DynamicFiles> for ServiceGroup {
    fn from(slf: DynamicFiles) -> ServiceGroup {
        let mut files = HashMap::new();
        log::info!("Searching for files at: {:?}", &slf.root_directory);
        if !slf.root_directory.exists() {
            if let Err(e) = std::fs::create_dir(&slf.root_directory) {
                log::error!("Error Creating Root Directory: {e:?}");
            }
        }
        if let Err(e) = read_directory(&slf.root_directory, slf.root_directory.clone(), &mut files)
        {
            log::error!("Error Loading files: {e:?}");
        }
        ServiceGroup {
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
                            cache_threshold: 65536,
                            cache_status: AtomicBool::default(),
                            cached_value: Arc::new(RwLock::new(Vec::with_capacity(0))),
                        }))
                        .build()
                })
                .collect(),
            shared_state: Default::default(),
        }
    }
}

pub struct FileLoader {
    pub name: String,
    pub mime: String,
    pub path: PathBuf,
    pub editable: bool,
    pub cache_threshold: u64,
    pub cache_status: AtomicBool,
    pub cached_value: Arc<RwLock<Vec<u8>>>,
}

#[async_trait::async_trait]
impl ServiceHandler for FileLoader {
    fn name(&self) -> &str {
        &self.name
    }
    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, (ServiceData, Error)> {
        if self.cache_status.load(Ordering::Relaxed) {
            if let Ok(val) = HeaderValue::from_str(&self.mime) {
                data.response.headers_mut().insert(CONTENT_TYPE, val);
            }
            let cached = self.cached_value.read().await.clone();
            data.response
                .headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from(cached.len()));
            data.response
                .set_body(BodyType::Stream(cached.stream_body()));
            Ok(data)
        } else {
            let mut stream = true;
            let file_path = if self.path.is_dir() {
                self.path.join("index.html")
            } else {
                self.path.clone()
            };
            match File::open(&file_path).await {
                Ok(f) => {
                    if let Ok(metadata) = f.metadata().await {
                        let size = metadata.len();
                        data.response
                            .headers_mut()
                            .insert(CONTENT_LENGTH, HeaderValue::from(size));
                        if size < self.cache_threshold {
                            match load_from_disk(&file_path).await {
                                Ok(bytes) => {
                                    *self.cached_value.write().await = bytes;
                                    self.cache_status.store(true, Ordering::Relaxed);
                                    stream = false;
                                }
                                Err(e) => {
                                    let err = format!("{e:?}");
                                    let bytes: Bytes = err.into();
                                    *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                                    data.response
                                        .set_body(BodyType::Stream(bytes.stream_body()));
                                    return Ok(data);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    let err = format!("{e:?}");
                    let bytes: Bytes = err.into();
                    *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    data.response
                        .set_body(BodyType::Stream(bytes.stream_body()));
                    return Ok(data);
                }
            }
            if stream {
                match stream_from_disk(&file_path).await {
                    Ok(stream) => {
                        if let Ok(val) = HeaderValue::from_str(&self.mime) {
                            data.response.headers_mut().insert(CONTENT_TYPE, val);
                        }
                        data.response.set_body(BodyType::Stream(stream));
                        Ok(data)
                    }
                    Err(e) => {
                        let err = format!("{e:?}");
                        let bytes: Bytes = err.into();
                        *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                        data.response
                            .set_body(BodyType::Stream(bytes.stream_body()));
                        return Ok(data);
                    }
                }
            } else {
                if let Ok(val) = HeaderValue::from_str(&self.mime) {
                    data.response.headers_mut().insert(CONTENT_TYPE, val);
                }
                let cached = self.cached_value.read().await.clone();
                data.response
                    .headers_mut()
                    .insert(CONTENT_LENGTH, HeaderValue::from(cached.len()));
                data.response
                    .set_body(BodyType::Stream(cached.stream_body()));
                Ok(data)
            }
        }
    }

    fn is_editable(&self) -> bool {
        true
    }

    fn service_type(&self) -> ServiceType {
        if self.path.is_dir() {
            ServiceType::Folder
        } else if self.path.is_file() {
            ServiceType::File
        } else {
            ServiceType::API
        }
    }

    async fn current_value(&self) -> EditResult {
        match load_from_disk(&self.path).await {
            Ok(bytes) => EditResult::Success(bytes),
            Err(e) => {
                let err = format!("{e:?}");
                EditResult::Failed(err)
            }
        }
    }

    async fn update_value(&self, new_value: Vec<u8>, current_value: Option<Vec<u8>>) -> EditResult {
        if let Some(to_match) = current_value {
            match load_from_disk(&self.path).await {
                Ok(disk_value) => {
                    if disk_value != to_match {
                        return EditResult::Failed(
                            "Expected Current Value does not match. File has been updated."
                                .to_string(),
                        );
                    }
                }
                Err(e) => {
                    return EditResult::Failed(format!("{e:?}"));
                }
            }
        }
        match OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&self.path)
            .await
        {
            Ok(mut file) => match file.write_all(&new_value).await {
                Ok(_) => EditResult::Success(new_value),
                Err(e) => EditResult::Failed(format!("{e:?}")),
            },
            Err(e) => EditResult::Failed(format!("{e:?}")),
        }
    }
}

async fn load_from_disk<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, Error> {
    tokio::fs::read(path).await
}

async fn stream_from_disk<P: AsRef<Path>>(path: P) -> Result<StreamingBody, Error> {
    let file = File::open(path).await?;
    let buffer = tokio_util::codec::FramedRead::new(file, BytesCodec::new())
        .map_ok(|b| Frame::data(Bytes::from(b.to_vec())))
        .map_err(|_| "Failed to Convert File to Stream");
    let stream = StreamBody::new(buffer);
    Ok(StreamBody::new(BodyStream::new(Box::pin(stream))))
}

pub struct StaticFile {
    pub name: &'static str,
    pub mime: String,
    pub file_contents: &'static [u8],
}
#[async_trait::async_trait]
impl ServiceHandler for StaticFile {
    fn name(&self) -> &str {
        self.name
    }
    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, (ServiceData, Error)> {
        let bytes: Bytes = self.file_contents.into();
        if let Ok(val) = HeaderValue::from_str(&self.mime) {
            data.response.headers_mut().insert(CONTENT_TYPE, val);
        }
        data.response
            .set_body(BodyType::Stream(bytes.stream_body()));
        Ok(data)
    }

    fn service_type(&self) -> ServiceType {
        ServiceType::File
    }
}

pub fn get_mime_type<P: AsRef<Path>>(path: P) -> String {
    from_path(path)
        .first_or_octet_stream() // Picks the first MIME type if multiple are guessed, or defaults to 'application/octet-stream'
        .to_string()
}
pub fn read_directory(
    root: &Path,
    file_path: PathBuf,
    file_map: &mut HashMap<String, PathBuf>,
) -> Result<(), Error> {
    for results in file_path.read_dir()? {
        match results {
            Ok(entry) => {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    read_directory(root, entry_path, file_map)?;
                } else {
                    read_file(root, entry_path, file_map)?;
                }
            }
            Err(e) => {
                log::error!("Error Loading file: {e:?}");
            }
        }
    }
    let mut new_root = std::path::PathBuf::from("/");
    let path = file_path.canonicalize()?;
    let path = path
        .strip_prefix(root)
        .map_err(|e| Error::new(::std::io::ErrorKind::InvalidInput, format!("{e:?}")))?;
    new_root.extend(path);
    file_map.insert(
        new_root.to_string_lossy().to_string(),
        file_path.join("index.html"),
    );
    Ok(())
}
pub fn read_file(
    root: &'_ Path,
    starting_path: PathBuf,
    file_map: &'_ mut HashMap<String, PathBuf>,
) -> Result<(), Error> {
    let mut new_root = std::path::PathBuf::from("/");
    let path = starting_path.canonicalize()?;
    let path = path
        .strip_prefix(root)
        .map_err(|e| Error::new(::std::io::ErrorKind::InvalidInput, format!("{e:?}")))?;
    new_root.extend(path);
    file_map.insert(new_root.to_string_lossy().to_string(), starting_path);
    Ok(())
}
