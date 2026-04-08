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
