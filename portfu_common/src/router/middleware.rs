use crate::error::PortfuError;
use crate::service::request::Request;
use crate::service::response::Response;
use std::pin::Pin;

pub mod client_trust;

pub enum MiddlewareResult {
    Continue,
    Return(Response),
}

pub trait Middleware {
    fn name(&self) -> &str;
    #[cfg(feature = "sessions")]
    fn session_manager(&self) -> Option<&crate::wrappers::sessions::SessionManager> {
        None
    }
    /// Identifies session middleware without relying on its display name.
    fn is_session_manager(&self) -> bool {
        false
    }
    fn before<'a>(
        &'a self,
        data: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>;
    #[allow(unused_variables)]
    fn after_with_request<'a>(
        &'a self,
        request: &'a Request,
        data: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        self.after(data)
    }
    fn after<'a>(
        &'a self,
        data: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>;
}
