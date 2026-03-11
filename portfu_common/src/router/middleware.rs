use crate::error::PortfuError;
use crate::service::request::Request;
use crate::service::response::Response;
use std::pin::Pin;

pub enum MiddlewareResult {
    Continue,
    Return(Response),
}

pub trait Middleware {
    fn name(&self) -> &str;
    fn before<'a>(
        &'a self,
        data: &mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>;
    fn after<'a>(
        &'a self,
        data: &mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>;
}
