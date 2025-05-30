use portfu::macros::{get, put};
use portfu::pfcore::editable::EditResult;
use portfu::pfcore::{FromBody, Json, ServiceRegister};
use portfu::prelude::http::{Extensions, StatusCode};
use portfu::prelude::*;
use serde::Deserialize;
use std::io::{Error, ErrorKind};
use std::sync::Arc;

#[get("/pf_admin/editor/files")]
pub async fn list_editable_files(data: &mut ServiceData) -> Result<Vec<u8>, Error> {
    let mut editable = vec![];
    let services: Vec<Arc<Service>> = data.server.registry.read().await.services.to_vec();
    for service in &services {
        if let Some(handle) = &service.handler {
            if handle.is_editable() {
                editable.push(service.name());
            }
        }
    }
    serde_json::to_vec(&editable).map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("Failed to Convert to JSON: {e:?}"),
        )
    })
}

#[get("/pf_admin/editor/folders")]
pub async fn list_editable_folders(data: &mut ServiceData) -> Result<Vec<u8>, Error> {
    let mut editable = vec![];
    let services: Vec<Arc<Service>> = data.server.registry.read().await.services.to_vec();
    for service in &services {
        if let Some(handle) = &service.handler {
            if handle.is_editable() {
                editable.push(service.name());
            }
        }
    }
    serde_json::to_vec(&editable).map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("Failed to Convert to JSON: {e:?}"),
        )
    })
}

#[derive(Deserialize)]
pub struct LoadRequest {
    service_name: String,
}

#[get("/pf_admin/editor/load")]
pub async fn get_service_value(data: &mut ServiceData) -> Result<Vec<u8>, Error> {
    let load_request: Option<LoadRequest> = Json::from_body(&mut data.request.request.body())
        .await?
        .inner();
    if let Some(load_request) = load_request {
        let services: Vec<Arc<Service>> = data.server.registry.read().await.services.to_vec();
        for service in &services {
            if service.name == load_request.service_name {
                if let Some(handle) = service.handler.clone() {
                    if handle.is_editable() {
                        match handle.current_value().await {
                            EditResult::Failed(s) => {
                                *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                                return Ok(s.into_bytes());
                            }
                            EditResult::Success(v) => {
                                return Ok(v);
                            }
                            EditResult::NotEditable => {
                                *data.response.status_mut() = StatusCode::FORBIDDEN;
                                return Ok(vec![]);
                            }
                        }
                    } else {
                        *data.response.status_mut() = StatusCode::FORBIDDEN;
                        return Ok(vec![]);
                    }
                }
            }
        }
    }
    *data.response.status_mut() = StatusCode::NOT_FOUND;
    Ok(vec![])
}

#[derive(Deserialize)]
pub struct EditRequest {
    service_name: String,
    new_value: Vec<u8>,
    current_value: Option<Vec<u8>>,
}

#[put("/pf_admin/editor/create")]
pub async fn create_service(data: &mut ServiceData) -> Result<Vec<u8>, Error> {
    let edit_request: Option<EditRequest> = Json::from_body(&mut data.request.request.body())
        .await?
        .inner();
    if let Some(edit_request) = edit_request {
        let services: Vec<Arc<Service>> = data.server.registry.read().await.services.to_vec();
        for service in &services {
            if service.name == edit_request.service_name {
                if let Some(handle) = service.handler.clone() {
                    if handle.is_editable() {
                        match handle
                            .update_value(edit_request.new_value, edit_request.current_value)
                            .await
                        {
                            EditResult::Failed(s) => {
                                *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                                return Ok(s.into_bytes());
                            }
                            EditResult::Success(v) => {
                                return Ok(v);
                            }
                            EditResult::NotEditable => {
                                *data.response.status_mut() = StatusCode::FORBIDDEN;
                                return Ok(vec![]);
                            }
                        }
                    } else {
                        *data.response.status_mut() = StatusCode::FORBIDDEN;
                        return Ok(vec![]);
                    }
                }
            }
        }
    }
    *data.response.status_mut() = StatusCode::NOT_FOUND;
    Ok(vec![])
}

#[put("/pf_admin/editor/update")]
pub async fn update_service_value(data: &mut ServiceData) -> Result<Vec<u8>, Error> {
    let edit_request: Option<EditRequest> = Json::from_body(&mut data.request.request.body())
        .await?
        .inner();
    if let Some(edit_request) = edit_request {
        let services: Vec<Arc<Service>> = data.server.registry.read().await.services.to_vec();
        for service in &services {
            if service.name == edit_request.service_name {
                if let Some(handle) = service.handler.clone() {
                    if handle.is_editable() {
                        match handle
                            .update_value(edit_request.new_value, edit_request.current_value)
                            .await
                        {
                            EditResult::Failed(s) => {
                                *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                                return Ok(s.into_bytes());
                            }
                            EditResult::Success(v) => {
                                return Ok(v);
                            }
                            EditResult::NotEditable => {
                                *data.response.status_mut() = StatusCode::FORBIDDEN;
                                return Ok(vec![]);
                            }
                        }
                    } else {
                        *data.response.status_mut() = StatusCode::FORBIDDEN;
                        return Ok(vec![]);
                    }
                }
            }
        }
    }
    *data.response.status_mut() = StatusCode::NOT_FOUND;
    Ok(vec![])
}

pub struct ServiceEditor {
    services: ServiceGroup,
}
impl Default for ServiceEditor {
    fn default() -> Self {
        Self {
            services: ServiceGroup::default()
                .service(list_editable_files)
                .service(list_editable_folders)
                .service(get_service_value)
                .service(update_service_value),
        }
    }
}
impl ServiceRegister for ServiceEditor {
    fn register(self, service_registry: &mut ServiceRegistry, shared_state: Extensions) {
        self.services.register(service_registry, shared_state);
    }
}
impl From<ServiceEditor> for ServiceGroup {
    fn from(value: ServiceEditor) -> Self {
        value.services
    }
}
