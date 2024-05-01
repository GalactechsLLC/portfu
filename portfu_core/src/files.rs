use crate::{IntoStreamBody, ServiceData, ServiceHandler};
use http::header::CONTENT_TYPE;
use http::HeaderValue;
use mime_guess::from_path;
use std::collections::HashMap;
use std::io::Error;
use std::path::Path;

pub struct FileLoader {
    pub name: String,
    pub mime: String,
    pub path: String,
}

#[async_trait::async_trait]
impl ServiceHandler for FileLoader {
    fn name(&self) -> &str {
        &self.name
    }
    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, Error> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(t) => {
                let bytes: hyper::body::Bytes = t.into();
                if let Ok(val) = HeaderValue::from_str(&self.mime) {
                    data.response.headers_mut().insert(CONTENT_TYPE, val);
                }
                *data.response.body_mut() = bytes.stream_body();
                Ok(data)
            }
            Err(e) => {
                let err = format!("{e:?}");
                let bytes: hyper::body::Bytes = err.into();
                *data.response.body_mut() = bytes.stream_body();
                Ok(data)
            }
        }
    }
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
    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, Error> {
        let bytes: hyper::body::Bytes = self.file_contents.into();
        if let Ok(val) = HeaderValue::from_str(&self.mime) {
            data.response.headers_mut().insert(CONTENT_TYPE, val);
        }
        *data.response.body_mut() = bytes.stream_body();
        Ok(data)
    }
}

pub fn get_mime_type<P: AsRef<Path>>(path: P) -> String {
    from_path(path)
        .first_or_octet_stream() // Picks the first MIME type if multiple are guessed, or defaults to 'application/octet-stream'
        .to_string()
}
pub fn read_directory(
    root: &Path,
    file_path: &Path,
    file_map: &mut HashMap<String, String>,
) -> Result<(), Error> {
    for results in file_path.read_dir()? {
        match results {
            Ok(entry) => {
                let entry_path = entry.path();
                if entry.path().is_dir() {
                    read_directory(root, entry_path.as_path(), file_map)?;
                } else {
                    read_file(root, entry_path.as_path(), file_map)?;
                }
            }
            Err(e) => {
                log::error!("Error Loading file: {e:?}");
            }
        }
    }
    Ok(())
}
pub fn read_file(
    root: &'_ Path,
    starting_path: &'_ Path,
    file_map: &'_ mut HashMap<String, String>,
) -> Result<(), Error> {
    let mut new_root = std::path::PathBuf::from("/");
    let path = starting_path.canonicalize()?;
    let path = path
        .strip_prefix(root)
        .map_err(|e| Error::new(::std::io::ErrorKind::InvalidInput, format!("{e:?}")))?;
    new_root.extend(path);
    file_map.insert(
        new_root.to_string_lossy().to_string(),
        starting_path.to_string_lossy().to_string(),
    );
    Ok(())
}
