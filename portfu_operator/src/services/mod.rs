use crate::services::kube::*;
use portfu::prelude::ServerBuilder;

pub mod kube;

pub fn register_services(server: ServerBuilder) -> ServerBuilder {
    server
        .register(get_nodes)
        .register(get_ingress)
        .register(get_services)
        .register(get_configs)
        .register(get_volume_claims)
        .register(get_pods)
        .register(get_volumes)
        .register(get_storage_classes)
        .register(get_namespaces)
}
