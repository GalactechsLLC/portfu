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

#[cfg(test)]
mod tests {
    use super::ClientTrust;
    use crate::router::middleware::{Middleware, MiddlewareResult};
    use crate::router::route::Route;
    use crate::server::connection::ClientIdentity;
    use crate::service::request::{Request, RequestType};
    use http::Method;
    use http_body_util::Full;
    use hyper::body::Bytes;
    use std::sync::Arc;

    fn request(method: Method, identity: Option<ClientIdentity>) -> Request {
        let request = http::Request::builder()
            .method(method)
            .uri("/private")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let mut request = Request::new(
            RequestType::Sized(request),
            Arc::new(Route::new("/private".to_string())),
        );
        if let Some(identity) = identity {
            request.insert(identity);
        }
        request
    }

    fn identity(verified_by: &[&str]) -> ClientIdentity {
        ClientIdentity {
            leaf_der: Arc::from([]),
            chain_der: Arc::from([]),
            sha256_fingerprint: [0; 32],
            verified_by: verified_by.iter().map(|value| value.to_string()).collect(),
        }
    }

    #[tokio::test]
    async fn missing_and_wrong_client_trust_return_distinct_statuses() {
        let middleware = ClientTrust::new("internal-clients");
        let missing = middleware
            .before(&mut request(Method::POST, None))
            .await
            .unwrap();
        let wrong = middleware
            .before(&mut request(
                Method::POST,
                Some(identity(&["public-clients"])),
            ))
            .await
            .unwrap();
        let allowed = middleware
            .before(&mut request(
                Method::POST,
                Some(identity(&["internal-clients"])),
            ))
            .await
            .unwrap();

        assert!(
            matches!(missing, MiddlewareResult::Return(response) if response.status() == http::StatusCode::UNAUTHORIZED)
        );
        assert!(
            matches!(wrong, MiddlewareResult::Return(response) if response.status() == http::StatusCode::FORBIDDEN)
        );
        assert!(matches!(allowed, MiddlewareResult::Continue));
    }

    #[tokio::test]
    async fn client_trust_preserves_options_preflight() {
        let result = ClientTrust::new("internal-clients")
            .before(&mut request(Method::OPTIONS, None))
            .await
            .unwrap();
        assert!(matches!(result, MiddlewareResult::Continue));
    }
}
