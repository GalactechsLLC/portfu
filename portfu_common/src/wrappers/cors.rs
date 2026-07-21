use crate::error::PortfuError;
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::service::request::Request;
use crate::service::response::Response;
use http::{HeaderName, HeaderValue};
use log::error;
use std::future::Future;
use std::pin::Pin;

const ACCESS_CONTROL_ALLOW_CREDENTIALS: &str = "access-control-allow-credentials";
const ACCESS_CONTROL_ALLOW_HEADERS: &str = "access-control-allow-headers";
const ACCESS_CONTROL_ALLOW_METHODS: &str = "access-control-allow-methods";
const ACCESS_CONTROL_ALLOW_ORIGIN: &str = "access-control-allow-origin";
const ACCESS_CONTROL_REQUEST_HEADERS: &str = "access-control-request-headers";
const ORIGIN: &str = "origin";
const VARY: &str = "vary";

pub struct Cors {
    allow_all: bool,
    allow_credentials: bool,
    allowed_origins: Vec<String>,
    allowed_methods: Vec<String>,
    allowed_headers: Vec<HeaderName>,
}

impl Cors {
    pub fn new(
        allowed_origins: Vec<String>,
        allowed_methods: Vec<String>,
        allowed_headers: Vec<HeaderName>,
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

    fn apply_headers(&self, request: &Request, response: &mut Response) {
        response.headers_mut().insert(
            HeaderName::from_static(ACCESS_CONTROL_ALLOW_CREDENTIALS),
            HeaderValue::from_static(if self.allow_credentials {
                "true"
            } else {
                "false"
            }),
        );

        if self.allow_all {
            response.headers_mut().insert(
                HeaderName::from_static(ACCESS_CONTROL_ALLOW_ORIGIN),
                HeaderValue::from_static("*"),
            );
            response.headers_mut().insert(
                HeaderName::from_static(ACCESS_CONTROL_ALLOW_METHODS),
                HeaderValue::from_static("*"),
            );
            response.headers_mut().insert(
                HeaderName::from_static(ACCESS_CONTROL_ALLOW_HEADERS),
                HeaderValue::from_static("*"),
            );
            return;
        }

        let Some(origin) = request.headers().get(ORIGIN) else {
            return;
        };
        let origin_str = match origin.to_str() {
            Ok(origin) => origin,
            Err(_) => return,
        };
        if !self
            .allowed_origins
            .iter()
            .any(|allowed| allowed == origin_str)
        {
            return;
        }

        response.headers_mut().insert(
            HeaderName::from_static(ACCESS_CONTROL_ALLOW_ORIGIN),
            origin.clone(),
        );
        response.headers_mut().insert(
            HeaderName::from_static(VARY),
            HeaderValue::from_static("Origin"),
        );

        if !self.allowed_methods.is_empty() {
            match HeaderValue::from_str(&self.allowed_methods.join(",")) {
                Ok(value) => {
                    response
                        .headers_mut()
                        .insert(HeaderName::from_static(ACCESS_CONTROL_ALLOW_METHODS), value);
                }
                Err(e) => {
                    error!("Error parsing allowed CORS methods: {e:?}");
                }
            }
        }

        let allowed_headers = self.allowed_request_headers(request);
        if !allowed_headers.is_empty() {
            match HeaderValue::from_str(&allowed_headers.join(",")) {
                Ok(value) => {
                    response
                        .headers_mut()
                        .insert(HeaderName::from_static(ACCESS_CONTROL_ALLOW_HEADERS), value);
                }
                Err(e) => {
                    error!("Error parsing allowed CORS headers: {e:?}");
                }
            }
        }
    }

    fn allowed_request_headers(&self, request: &Request) -> Vec<String> {
        let Some(requested_headers) = request.headers().get(ACCESS_CONTROL_REQUEST_HEADERS) else {
            return Vec::new();
        };
        let requested_headers = match requested_headers.to_str() {
            Ok(headers) => headers,
            Err(_) => return Vec::new(),
        };

        requested_headers
            .split(',')
            .map(str::trim)
            .filter(|header| !header.is_empty())
            .filter_map(|header| {
                let header_name = HeaderName::from_bytes(header.as_bytes()).ok()?;
                self.allowed_headers
                    .contains(&header_name)
                    .then(|| header_name.as_str().to_string())
            })
            .collect()
    }
}

impl Middleware for Cors {
    fn name(&self) -> &str {
        "Cors Wrapper"
    }

    fn before<'a>(
        &'a self,
        _request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move { Ok(MiddlewareResult::Continue) })
    }

    fn after_with_request<'a>(
        &'a self,
        request: &'a Request,
        response: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move {
            self.apply_headers(request, response);
            Ok(MiddlewareResult::Continue)
        })
    }

    fn after<'a>(
        &'a self,
        _response: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move { Ok(MiddlewareResult::Continue) })
    }
}
