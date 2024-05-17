use std::io::{Error, ErrorKind};
use std::sync::Arc;
use serde::Deserialize;
use pfcore::{Body, FromRequest, IntoStreamBody, ServiceData, ServiceHandler, ServiceRegister, ServiceRegistry};
use pfcore::service::{ServiceBuilder};
use crate::filters::method::GET;

#[derive(Deserialize)]
pub struct EditRequest {
    
}

pub struct EditHandler {}
impl ServiceRegister for EditHandler {
    fn register(self, service_registry: &mut ServiceRegistry) {
        let service = ServiceBuilder::new("/editable")
            .name("index")
            .filter(GET.clone())
            .handler(Arc::new(EditHandler {}))
            .build();
        service_registry.register(service);
    }
}
#[async_trait::async_trait]
impl ServiceHandler for EditHandler {
    fn name(&self) -> &str {
        "Edit Manager"
    }
    async fn handle(&self, mut data: ServiceData) -> Result<ServiceData, (ServiceData, Error)> {
        match Body::from_request(&mut data.request, "").await {
            Ok(body) => {
                let body: Vec<u8> = body.inner();
                match serde_json::from_slice::<EditRequest>(body.as_slice()) {
                    Ok(_json) => {
                        let mut editable = vec![];
                        for service in &data.server.services.services {
                            if service.editable.is_some(){
                                editable.push(service.name.clone());
                            }
                        }
                        *data.response.body_mut() = serde_json::to_vec(&editable).unwrap_or_default().stream_body();
                        Ok(data)
                    }
                    Err(e) => {
                        Err((data, Error::new(ErrorKind::InvalidData, format!("{e:?}"))))
                    }
                }
            }
            Err(e) => {
                Err((data, e))
            }
        }
    }
}