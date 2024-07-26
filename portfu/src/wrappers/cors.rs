use async_trait::async_trait;
use http::{HeaderName, HeaderValue};
use pfcore::wrappers::{WrapperFn, WrapperResult};
use pfcore::ServiceData;

pub struct Cors {
    allow_all: bool,
    allow_credentials: bool,
    allowed_origins: Vec<String>,
    allowed_methods: Vec<String>,
    allowed_headers: Vec<String>,
}
impl Cors {
    pub fn new(
        allowed_origins: Vec<String>,
        allowed_methods: Vec<String>,
        allowed_headers: Vec<String>,
        allow_credentials: bool,
    ) -> Self {
        Self {
            allow_all: false,
            allow_credentials,
            allowed_origins,
            allowed_methods,
            allowed_headers,
        }
    }
    pub fn allow_all() -> Self {
        Self {
            allow_all: true,
            allow_credentials: false,
            allowed_origins: vec![],
            allowed_methods: vec![],
            allowed_headers: vec![],
        }
    }
}
#[async_trait]
impl WrapperFn for Cors {
    fn name(&self) -> &str {
        "Cors Wrapper"
    }

    async fn before(&self, _: &mut ServiceData) -> WrapperResult {
        WrapperResult::Continue
    }

    async fn after(&self, data: &mut ServiceData) -> WrapperResult {
        if self.allow_credentials {
            data.response.headers_mut().insert(
                HeaderName::from_static("access-control-allow-credentials"),
                HeaderValue::from_static("true"),
            );
        } else {
            data.response.headers_mut().insert(
                HeaderName::from_static("access-control-allow-credentials"),
                HeaderValue::from_static("false"),
            );
        }
        if self.allow_all {
            data.response.headers_mut().insert(
                HeaderName::from_static("access-control-allow-origin"),
                HeaderValue::from_static("*"),
            );
            data.response.headers_mut().insert(
                HeaderName::from_static("access-control-allow-methods"),
                HeaderValue::from_static("*"),
            );
            data.response.headers_mut().insert(
                HeaderName::from_static("access-control-allow-headers"),
                HeaderValue::from_static("*"),
            );
        } else if let Some(headers) = data.request.request.headers() {
            if let Some(origin) = headers.get("origin") {
                if self
                    .allowed_origins
                    .contains(&origin.to_str().unwrap_or_default().to_string())
                {
                    data.response.headers_mut().insert(
                        HeaderName::from_static("access-control-allow-origin"),
                        origin.clone(),
                    );
                    if let Ok(val) = HeaderValue::from_str(&self.allowed_methods.join(",")) {
                        data.response
                            .headers_mut()
                            .insert(HeaderName::from_static("access-control-allow-methods"), val);
                    }
                    if let Ok(val) = HeaderValue::from_str(&self.allowed_headers.join(",")) {
                        data.response
                            .headers_mut()
                            .insert(HeaderName::from_static("access-control-allow-headers"), val);
                    }
                }
            }
        }
        WrapperResult::Continue
    }
}
