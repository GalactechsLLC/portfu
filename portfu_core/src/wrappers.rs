use async_trait::async_trait;
use http::Response;
use http_body_util::Full;
use hyper::body::Bytes;
use crate::service::ServiceRequest;
use crate::ServiceResponse;

#[async_trait]
pub trait Wrapper {
    fn name(&self) -> &str;
    async fn before(
        &self,
        request: ServiceRequest,
        response: Response<Full<Bytes>>
    ) -> Result<ServiceResponse, ServiceResponse> {
        Ok(ServiceResponse {
            request,
            response,
        })
    }
    async fn after(
        &self,
        request: ServiceRequest,
        response: Response<Full<Bytes>>
    ) -> Result<ServiceResponse, ServiceResponse>{
        Ok(ServiceResponse {
            request,
            response,
        })
    }
}