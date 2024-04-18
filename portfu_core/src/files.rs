use crate::{ServiceData, ServiceHandler};
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

pub fn get_mime_type<P: AsRef<Path>>(path: P) -> String {
    from_path(path)
        .first_or_octet_stream() // Picks the first MIME type if multiple are guessed, or defaults to 'application/octet-stream'
        .to_string()
}

#[async_trait::async_trait]
impl ServiceHandler for FileLoader {
    fn name(&self) -> &str {
        &self.name
    }
    async fn handle(&self, data: &mut ServiceData) -> Result<(), Error> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(t) => {
                let bytes: hyper::body::Bytes = t.into();
                if let Ok(val) = HeaderValue::from_str(&self.mime) {
                    data.response.headers_mut().insert(CONTENT_TYPE, val);
                }
                *data.response.body_mut() = http_body_util::Full::new(bytes);
                Ok(())
            }
            Err(e) => {
                let err = format!("{e:?}");
                let bytes: hyper::body::Bytes = err.into();
                *data.response.body_mut() = http_body_util::Full::new(bytes);
                Ok(())
            }
        }
    }
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
