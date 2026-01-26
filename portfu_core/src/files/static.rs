use crate::services::body::BodyType;
use crate::{IntoStreamBody, ServiceData, ServiceHandler, ServiceType};
use http::header::CONTENT_TYPE;
use http::HeaderValue;
use hyper::body::Bytes;
use log::debug;
use std::io::Error;

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
        debug!("mime type: {}, path: {}", self.mime, self.name);
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
