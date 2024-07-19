use std::io::{Error, ErrorKind};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{ConfigMap, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod, Service as KubeService};
use k8s_openapi::api::networking::v1::Ingress;
use k8s_openapi::api::storage::v1::StorageClass;
use portfu::macros::get;
use portfu::prelude::*;
use kube::{Api, Client};
use kube::api::{ObjectList};
use portfu::pfcore::Json;
use crate::kube::SearchParams;

pub struct KubeNamespace(pub String);

#[get("/cluster/nodes", output = "Json")]
pub async fn get_nodes(client: State<Client>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<Node>, Error>{
    let api: Api<Node> = Api::all(client.as_ref().clone());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading Node List: {e:?}"))
    })
}

#[get("/cluster/ingress", output = "Json")]
pub async fn get_ingress(client: State<Client>, namespace: State<KubeNamespace>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<Ingress>, Error>{
    let api: Api<Ingress> = Api::namespaced(client.as_ref().clone(), namespace.as_ref().0.as_ref());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading Ingress List: {e:?}"))
    })
}

#[get("/cluster/services", output = "Json")]
pub async fn get_services(client: State<Client>, namespace: State<KubeNamespace>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<KubeService>, Error>{
    let api: Api<KubeService> = Api::namespaced(client.as_ref().clone(), namespace.as_ref().0.as_ref());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading Service List: {e:?}"))
    })
}

#[get("/cluster/configs", output = "Json")]
pub async fn get_configs(client: State<Client>, namespace: State<KubeNamespace>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<ConfigMap>, Error>{
    let api: Api<ConfigMap> = Api::namespaced(client.as_ref().clone(), namespace.as_ref().0.as_ref());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading Config Map List: {e:?}"))
    })
}

#[get("/cluster/volume_claims", output = "Json")]
pub async fn get_volume_claims(client: State<Client>, namespace: State<KubeNamespace>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<PersistentVolumeClaim>, Error>{
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.as_ref().clone(), namespace.as_ref().0.as_ref());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading PersistentVolumeClaim List: {e:?}"))
    })
}

#[get("/cluster/pods", output = "Json")]
pub async fn get_pods(client: State<Client>, namespace: State<KubeNamespace>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<Pod>, Error>{
    let api: Api<Pod> = Api::namespaced(client.as_ref().clone(), namespace.as_ref().0.as_ref());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading Pod List: {e:?}"))
    })
}

#[get("/cluster/volumes", output = "Json")]
pub async fn get_volumes(client: State<Client>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<PersistentVolume>, Error>{
    let api: Api<PersistentVolume> = Api::all(client.as_ref().clone());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading Volume List: {e:?}"))
    })
}

#[get("/cluster/storage_classes", output = "Json")]
pub async fn get_storage_classes(client: State<Client>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<StorageClass>, Error>{
    let api: Api<StorageClass> = Api::all(client.as_ref().clone());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading StorageClass List: {e:?}"))
    })
}

#[get("/cluster/namespaces", output = "Json")]
pub async fn get_namespaces(client: State<Client>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<Namespace>, Error>{
    let api: Api<Namespace> = Api::all(client.as_ref().clone());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading Namespace List: {e:?}"))
    })
}

#[get("/cluster/deployments", output = "Json")]
pub async fn get_deployments(client: State<Client>, list_params: Json<Option<SearchParams>>) -> Result<ObjectList<Deployment>, Error>{
    let api: Api<Deployment> = Api::all(client.as_ref().clone());
    api.list(&list_params.inner().unwrap_or_default().into()).await.map_err(|e| {
        Error::new(ErrorKind::Other, format!("Error Reading Deployment List: {e:?}"))
    })
}