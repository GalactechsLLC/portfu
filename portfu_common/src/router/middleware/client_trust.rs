use crate::error::PortfuError;
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::server::connection::ClientIdentity;
use crate::service::request::Request;
use crate::service::response::Response;
use http::{Method, StatusCode};
use std::pin::Pin;

#[derive(Clone, Debug)]
pub struct ClientTrust {
    trust_store: String,
}

impl ClientTrust {
    pub fn new(trust_store: impl Into<String>) -> Self {
        Self {
            trust_store: trust_store.into(),
        }
    }

    pub fn trust_store(&self) -> &str {
        &self.trust_store
    }
}

impl Middleware for ClientTrust {
    fn name(&self) -> &str {
        "ClientTrust"
    }

    fn before<'a>(
        &'a self,
        request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move {
            // Method macros expose OPTIONS for CORS preflight; it carries no protected payload.
            if request.method() == Method::OPTIONS {
                return Ok(MiddlewareResult::Continue);
            }
            let Some(identity) = request.get::<ClientIdentity>() else {
                return Ok(MiddlewareResult::Return(Response::from_status_and_message(
                    StatusCode::UNAUTHORIZED,
                    "Client certificate required",
                )));
            };
            if identity
                .verified_by
                .iter()
                .any(|name| name == &self.trust_store)
            {
                Ok(MiddlewareResult::Continue)
            } else {
                Ok(MiddlewareResult::Return(Response::from_status_and_message(
                    StatusCode::FORBIDDEN,
                    format!(
                        "Client certificate is not trusted by `{}`",
                        self.trust_store
                    ),
                )))
            }
        })
    }

    fn after<'a>(
        &'a self,
        _response: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async { Ok(MiddlewareResult::Continue) })
    }
}
