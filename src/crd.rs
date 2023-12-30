use k8s_openapi::api::admissionregistration::v1::{
    MutatingWebhookConfiguration, ValidatingWebhookConfiguration,
};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::core::v1::Service;
use kube::{CustomResource, ResourceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone)]
pub enum DeploymentType {
    Pod(Pod),
    Deployment(Deployment),
}

#[derive(Serialize, Deserialize, Clone)]
pub enum WebhookType {
    Mutating(MutatingWebhookConfiguration),
    Validating(ValidatingWebhookConfiguration),
}

#[derive(Serialize, Deserialize, Clone)]
pub enum Stage {
    HelperCreated,
    Deleting,
    CertificateCreated(String),
    ServiceCreated(Service),
    DeploymentStarted(DeploymentType),
    DeploymentComplete(DeploymentType),
    WebhookCreated(WebhookType),
    CreationFailed(String),
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let message: String = match self {
            Stage::HelperCreated => "WebhookHelperCreated".into(),
            Stage::CertificateCreated(_) => "CertificateCreated".into(),
            Stage::DeploymentComplete(_) => "DeploymentComplete".into(),
            Stage::DeploymentStarted(_) => "DeploymentStarted".into(),
            Stage::ServiceCreated(_) => "ServiceCreated".into(),
            Stage::WebhookCreated(_) => "WebhookCreated".into(),
            Stage::CreationFailed(_) => "CreationFailed".into(),
            Stage::Deleting => "Deleting".into(),
        };
        write!(f, "{}", message)
    }
}

impl Stage {
    pub fn message(&self) -> String {
        match self {
            Stage::HelperCreated => "Webhook Helper Created".to_string(),
            Stage::CertificateCreated(c) => format!("Certificate {} Created", c),
            Stage::DeploymentComplete(d) => match d {
                DeploymentType::Deployment(dep) => {
                    format!("Deployment {} Completed", dep.name_any())
                }
                DeploymentType::Pod(p) => format!("Deployment {} Completed", p.name_any()),
            },
            Stage::DeploymentStarted(d) => match d {
                DeploymentType::Deployment(dep) => format!("Deployment {} Started", dep.name_any()),
                DeploymentType::Pod(p) => format!("Deployment {} Started", p.name_any()),
            },
            Stage::ServiceCreated(s) => format!("Service {} Created", s.name_any()),
            Stage::WebhookCreated(s) => match s {
                WebhookType::Mutating(m) => format!("Webhook {} Created", m.name_any()),
                WebhookType::Validating(m) => format!("Webhook {} Created", m.name_any()),
            },
            Stage::CreationFailed(r) => format!("Webhook-helper failed to created webhook: {}", r),
            Stage::Deleting => "Deleting resource".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, JsonSchema, Default)]
pub struct WebhookHelperCondition {
    #[serde(rename = "type")]
    pub type__: String,
    pub message: String,
    pub status: String,
    #[serde(rename = "lastTransitionTime")]
    pub last_transition_time: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, JsonSchema, Default)]
pub struct WebhookHelperStatus {
    pub certificate: Option<String>,
    pub service: Option<String>,
    pub deployment: Option<String>,
    pub pod: Option<String>,
    pub validating_webhook: Option<String>,
    pub mutating_webhook: Option<String>,
    pub conditions: Option<Vec<WebhookHelperCondition>>,
}

#[derive(CustomResource, Serialize, Deserialize, Clone, PartialEq, Debug, JsonSchema)]
#[kube(group = "webhook-helper.io", version = "v1", kind = "WebhookHelper")]
#[kube(singular = "webhook-helper", plural = "webhook-helpers")]
#[kube(status = "WebhookHelperStatus")]
pub struct HelperSpec {
    pub namespace: String,
    pub webhook: Value,
    pub listening_port: i32,
    pub target_port: Option<i32>,
    pub path: Option<String>,
    pub container_name: Option<String>,
    pub deployment: Value,
}
