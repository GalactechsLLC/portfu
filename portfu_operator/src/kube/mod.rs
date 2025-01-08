use k8s_openapi::api::apps::v1::{
    Deployment, DeploymentSpec, DeploymentStrategy, RollingUpdateDeployment,
};
use k8s_openapi::api::core::v1::{
    ConfigMap, Namespace, PersistentVolumeClaim, PersistentVolumeClaimSpec, Pod,
    ResourceRequirements, Service, ServicePort, ServiceSpec,
};
use k8s_openapi::api::networking::v1::{
    HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
    IngressServiceBackend, IngressSpec, IngressTLS, ServiceBackendPort,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use k8s_openapi::serde_json::json;
use kube::api::{ListParams, Patch, PatchParams};
use kube::{Api, Client};
use log::warn;
use portfu::prelude::*;
use portfu_runtime_lib::config::Config as PortfuConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Error, ErrorKind};
use std::ops::Deref;

pub const DEFAULT_NAMESPACE: &str = "portfu-infrastructure";
#[allow(unused)]
pub const DEFAULT_CONFIG_PREFIX: &str = "portfu-config-";
#[allow(unused)]
pub const DEFAULT_POD_PREFIX: &str = "portfu-";
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SearchParams {
    pub label_selector: Option<String>,
    pub field_selector: Option<String>,
    pub limit: Option<u32>,
    pub continue_token: Option<String>,
}

impl From<SearchParams> for ListParams {
    fn from(value: SearchParams) -> Self {
        Self {
            label_selector: value.label_selector,
            field_selector: value.field_selector,
            limit: value.limit,
            continue_token: value.continue_token,
            ..Default::default()
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeConfig {
    pub read_only: bool,
    pub name: String,
    pub claim_name: Option<String>,
    pub storage_class: Option<String>,
    pub storage_size: Option<String>,
    pub mount_path: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostConfig {
    pub name: String,
    pub hostname: String,
    pub port: u16,
    pub tls: bool,
    pub paths: Vec<PathConfig>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathConfig {
    pub path: String,
    pub service_name: String,
    pub service_port: u16,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub port: u16,
    pub target_port: u16,
    pub tls: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngressConfig {
    pub name: Option<String>,
    pub services: Vec<ServiceConfig>,
    pub hosts: Vec<HostConfig>,
    pub class_name: Option<String>,
    pub annotations: BTreeMap<String, String>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub portfu: PortfuConfig,
    pub env_vars: Vec<(String, String)>,
    pub volumes: Vec<VolumeConfig>,
    pub ingress: IngressConfig,
    pub name: String,
    pub replicas: Option<i32>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfigMap(ConfigMap);
#[allow(unused)]
impl RuntimeConfigMap {
    pub fn name(&self) -> Option<&str> {
        self.0.metadata.name.as_deref()
    }
    pub fn uuid(&self) -> Option<&str> {
        if let Some(data) = &self.0.data {
            data.get("uuid").map(|s| s.as_str())
        } else {
            None
        }
    }
    pub fn pod_name(&self) -> Option<&str> {
        if let Some(data) = &self.0.data {
            data.get("pod_name").map(|s| s.as_str())
        } else {
            None
        }
    }
    pub fn managed(&self) -> bool {
        if let Some(data) = &self.0.data {
            data.get("managed")
                .map(|m| m.eq_ignore_ascii_case("true"))
                .unwrap_or_default()
        } else {
            false
        }
    }
    pub fn config(&self) -> Result<Option<RuntimeConfig>, Error> {
        if let Some(data) = &self.0.data {
            if let Some(config_json) = data.get("config.json") {
                serde_json::from_str(config_json)
                    .map_err(|e| {
                        Error::new(
                            ErrorKind::InvalidInput,
                            format!("Failed to parse Config JSON: {e:?}"),
                        )
                    })
                    .map(Some)
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}
impl Deref for RuntimeConfigMap {
    type Target = ConfigMap;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[allow(unused)]
pub async fn find_configs(
    client: Client,
    namespace: Option<String>,
) -> Result<Vec<RuntimeConfigMap>, Error> {
    let api: Api<ConfigMap> = Api::namespaced(
        client.clone(),
        namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE),
    );
    let lp = ListParams::default();
    let cm_list = api
        .list(&lp)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))?;
    let mut configs = vec![];
    for cm in cm_list {
        if let Some(s) = &cm.metadata.name {
            if s.starts_with(DEFAULT_CONFIG_PREFIX) {
                configs.push(RuntimeConfigMap(cm));
            }
        }
    }
    Ok(configs)
}

#[allow(unused)]
pub async fn save_config(
    client: Client,
    namespace: Option<String>,
    config_map: &RuntimeConfigMap,
) -> Result<RuntimeConfigMap, Error> {
    let api: Api<ConfigMap> = Api::namespaced(
        client.clone(),
        namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE),
    );
    let patch = Patch::Apply(json!(config_map));
    let pp = PatchParams::apply("portfu-operator");
    if let Some(name) = config_map.name() {
        let patched = api
            .patch(name, &pp, &patch)
            .await
            .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))?;
        Ok(RuntimeConfigMap(patched))
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            format!("ConfigMap does not have a name, unable to update {config_map:?}"),
        ))
    }
}

#[allow(unused)]
pub async fn create_namespace(
    client: Client,
    namespace: Option<String>,
) -> Result<Namespace, Error> {
    let namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let api: Api<Namespace> = Api::all(client.clone());
    let patch = Patch::Apply(json!(Namespace {
        metadata: ObjectMeta {
            annotations: None,
            creation_timestamp: None,
            deletion_grace_period_seconds: None,
            deletion_timestamp: None,
            finalizers: None,
            generate_name: None,
            generation: None,
            labels: None,
            managed_fields: None,
            name: Some(namespace.to_string()),
            namespace: Some(namespace.to_string()),
            owner_references: None,
            resource_version: None,
            self_link: None,
            uid: None,
        },
        spec: None,
        status: None,
    }));
    let pp = PatchParams::apply("portfu-operator");
    api.patch(namespace, &pp, &patch)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))
}

#[allow(unused)]
pub async fn create_persistent_volume_claim(
    client: Client,
    namespace: Option<String>,
    volume_config: &VolumeConfig,
) -> Result<PersistentVolumeClaim, Error> {
    let namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let api: Api<PersistentVolumeClaim> = Api::all(client.clone());
    let name = volume_config
        .claim_name
        .clone()
        .unwrap_or(format!("pvc-{}", volume_config.name));
    let patch = Patch::Apply(json!(PersistentVolumeClaim {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".into()]),
            resources: Some(ResourceRequirements {
                requests: Some(
                    [(
                        "storage".to_string(),
                        Quantity(
                            volume_config
                                .storage_size
                                .clone()
                                .unwrap_or("5Gi".to_string())
                        )
                    )]
                    .into()
                ),
                ..Default::default()
            }),
            storage_class_name: volume_config.storage_class.clone(),
            ..Default::default()
        }),
        ..Default::default()
    }));
    let pp = PatchParams::apply("portfu-operator");
    api.patch(&name, &pp, &patch)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))
}

#[allow(unused)]
pub async fn create_service(
    client: Client,
    namespace: Option<String>,
    runtime_config: &RuntimeConfig,
) -> Result<Service, Error> {
    let namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let api: Api<Service> = Api::all(client.clone());
    let patch = Patch::Apply(json!(Service {
        metadata: ObjectMeta {
            name: Some(runtime_config.name.clone()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            ports: if runtime_config.ingress.services.is_empty() {
                None
            } else {
                Some(
                    runtime_config
                        .ingress
                        .services
                        .iter()
                        .map(|v| ServicePort {
                            name: Some(v.name.clone()),
                            port: v.port as i32,
                            target_port: Some(IntOrString::Int(v.target_port as i32)),
                            ..Default::default()
                        })
                        .collect(),
                )
            },
            publish_not_ready_addresses: None,
            selector: Some(BTreeMap::from([(
                "app".to_string(),
                runtime_config.name.clone()
            )])),
            ..Default::default()
        }),
        ..Default::default()
    }));
    let pp = PatchParams::apply("portfu-operator");
    api.patch(&runtime_config.name, &pp, &patch)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))
}

#[allow(unused)]
pub async fn create_ingress(
    client: Client,
    namespace: Option<String>,
    runtime_config: &RuntimeConfig,
) -> Result<Ingress, Error> {
    let namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let api: Api<Ingress> = Api::namespaced(client.clone(), namespace);
    let name = runtime_config
        .ingress
        .name
        .clone()
        .unwrap_or(format!("ing-{}", runtime_config.name));
    let patch = Patch::Apply(json!(Ingress {
        metadata: ObjectMeta {
            annotations: if runtime_config.ingress.annotations.is_empty() {
                None
            } else {
                Some(runtime_config.ingress.annotations.clone())
            },
            labels: if runtime_config.ingress.labels.is_empty() {
                None
            } else {
                Some(runtime_config.ingress.labels.clone())
            },
            name: Some(name.clone()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(IngressSpec {
            default_backend: None,
            ingress_class_name: runtime_config.ingress.class_name.clone(),
            rules: if runtime_config.ingress.hosts.is_empty() {
                None
            } else {
                Some(
                    runtime_config
                        .ingress
                        .hosts
                        .iter()
                        .filter(|v| v.tls)
                        .map(|v| IngressRule {
                            host: Some(v.hostname.clone()),
                            http: Some(HTTPIngressRuleValue {
                                paths: v
                                    .paths
                                    .iter()
                                    .map(|p| HTTPIngressPath {
                                        backend: IngressBackend {
                                            resource: None,
                                            service: Some(IngressServiceBackend {
                                                name: p.service_name.clone(),
                                                port: Some(ServiceBackendPort {
                                                    name: None,
                                                    number: Some(p.service_port as i32),
                                                }),
                                            }),
                                        },
                                        path: Some(p.path.clone()),
                                        path_type: "ImplementationSpecific".to_string(),
                                    })
                                    .collect(),
                            }),
                        })
                        .collect(),
                )
            },
            tls: if runtime_config.ingress.hosts.is_empty()
                || runtime_config.ingress.hosts.iter().all(|v| !v.tls)
            {
                None
            } else {
                Some(
                    runtime_config
                        .ingress
                        .hosts
                        .iter()
                        .map(|v| IngressTLS {
                            hosts: Some(vec![v.hostname.clone()]),
                            secret_name: Some(format!("tls-{}", v.name)),
                        })
                        .collect(),
                )
            },
        }),
        ..Default::default()
    }));
    let pp = PatchParams::apply("portfu-operator");
    api.patch(&name, &pp, &patch)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))
}

#[allow(unused)]
pub async fn has_persistent_volume_claim(
    client: Client,
    namespace: Option<String>,
    volume_config: &VolumeConfig,
) -> Result<Option<PersistentVolumeClaim>, Error> {
    let namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    api.get_opt(&volume_config.name)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))
}

#[allow(unused)]
pub async fn has_service(
    client: Client,
    namespace: Option<String>,
    config: &RuntimeConfig,
) -> Result<Option<Service>, Error> {
    let namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let api: Api<Service> = Api::namespaced(client.clone(), namespace);
    api.get_opt(&config.name)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))
}

#[allow(unused)]
pub async fn has_running_pod(
    client: Client,
    namespace: Option<String>,
    config: &RuntimeConfig,
) -> Result<Option<Pod>, Error> {
    let namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod = api
        .get_opt(&config.name)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))?;
    Ok(pod)
}

#[allow(unused)]
pub async fn has_deployment(
    client: Client,
    namespace: Option<String>,
    config: &RuntimeConfig,
) -> Result<Option<Deployment>, Error> {
    let namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let deployment = api
        .get_opt(&config.name)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))?;
    Ok(deployment)
}

#[allow(unused)]
pub async fn has_ingress(
    client: Client,
    namespace: Option<String>,
    config: &RuntimeConfig,
) -> Result<Option<Ingress>, Error> {
    let namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let api: Api<Ingress> = Api::namespaced(client.clone(), namespace);
    let ingress = api
        .get_opt(&config.name)
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))?;
    Ok(ingress)
}

#[allow(unused)]
pub async fn create_deployment(
    client: Client,
    namespace: Option<String>,
    config_map: &RuntimeConfigMap,
) -> Result<Option<Deployment>, Error> {
    if let Some(config) = config_map.config()? {
        //Validate Deployment Volumes
        let mut found_pvc = vec![];
        for volume in &config.volumes {
            if let Some(pvc) =
                has_persistent_volume_claim(client.clone(), namespace.clone(), volume).await?
            {
                found_pvc.push(pvc);
            } else {
                let pvc = create_persistent_volume_claim(client.clone(), namespace.clone(), volume)
                    .await?;
                found_pvc.push(pvc);
            }
        }
        //Create the Deployment
        let d_namespace = namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
        let api: Api<Deployment> = Api::namespaced(client.clone(), d_namespace);
        let patch = Patch::Apply(json!(Deployment {
            metadata: Default::default(),
            spec: Some(DeploymentSpec {
                min_ready_seconds: None,
                paused: None,
                progress_deadline_seconds: None,
                replicas: None,
                revision_history_limit: None,
                selector: LabelSelector {
                    match_expressions: None,
                    match_labels: Some(BTreeMap::from([("app".to_string(), config.name.clone())])),
                },
                strategy: Some(DeploymentStrategy {
                    rolling_update: Some(RollingUpdateDeployment {
                        max_surge: Some(IntOrString::String("25%".to_string())),
                        max_unavailable: Some(IntOrString::String("25%".to_string())),
                    }),
                    type_: Some("RollingUpdate".to_string()),
                }),
                template: Default::default(),
            }),
            status: None,
        }));
        let pp = PatchParams::apply("portfu-operator");
        let deployment = api
            .patch(&config.name, &pp, &patch)
            .await
            .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))?;
        //Create the Services
        let _services =
            if let Some(i) = has_service(client.clone(), namespace.clone(), &config).await? {
                i
            } else {
                create_service(client.clone(), namespace.clone(), &config).await?
            };
        //Create the Ingress
        let _ingress =
            if let Some(i) = has_ingress(client.clone(), namespace.clone(), &config).await? {
                i
            } else {
                create_ingress(client.clone(), namespace.clone(), &config).await?
            };
        Ok(Some(deployment))
    } else {
        warn!("Has Running Deployment Called on ConfigMap with No Name");
        Ok(None)
    }
}

#[tokio::test]
pub async fn test_save_config() -> Result<(), Error> {
    let client = Client::try_default()
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))?;
    let uuid = uuid::Uuid::new_v4();
    let namespace = create_namespace(client.clone(), None).await?;
    let patched = save_config(
        client,
        None,
        &RuntimeConfigMap(ConfigMap {
            binary_data: None,
            data: Some(std::collections::BTreeMap::from([
                ("name".to_string(), format!("{DEFAULT_CONFIG_PREFIX}{uuid}")),
                (
                    "pod-name".to_string(),
                    format!("{DEFAULT_POD_PREFIX}{uuid}"),
                ),
                ("uuid".to_string(), uuid.to_string()),
                (
                    "config.json".to_string(),
                    serde_json::to_string(&PortfuConfig::default()).unwrap_or_default(),
                ),
            ])),
            immutable: Some(false),
            metadata: ObjectMeta {
                annotations: None,
                creation_timestamp: None,
                deletion_grace_period_seconds: None,
                deletion_timestamp: None,
                finalizers: None,
                generate_name: None,
                generation: None,
                labels: None,
                managed_fields: None,
                name: Some(format!("{DEFAULT_CONFIG_PREFIX}{uuid}")),
                namespace: Some(DEFAULT_NAMESPACE.to_string()),
                owner_references: None,
                resource_version: None,
                self_link: None,
                uid: None,
            },
        }),
    )
    .await?;
    println!("Namespace: {:?}", namespace);
    println!("ConfigMap: {:?}", patched);
    Ok(())
}

#[tokio::test]
pub async fn test_find_configs() -> Result<(), Error> {
    let client = Client::try_default()
        .await
        .map_err(|e| Error::new(ErrorKind::Other, format!("{e:?}")))?;
    let data = find_configs(client, Some("infrastructure".to_string())).await?;
    println!("Found: {}", data.len());
    Ok(())
}
