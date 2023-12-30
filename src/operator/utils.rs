use kube::api::{DeleteParams, PostParams};
use kube::core::ResourceExt;
use kube::Api;
use kube::Client;

use serde_json::Value;

use kube::api::Patch;
use kube::api::PatchParams;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Debug;

use crate::controller::Error;
use crate::crd::WebhookType;
use crate::crd::{
    DeploymentType, Stage, WebhookHelper, WebhookHelperCondition, WebhookHelperStatus,
};
use chrono::offset::Utc;
use chrono::DateTime;
use k8s_openapi::{ClusterResourceScope, NamespaceResourceScope};
use kube::Resource;
use std::time::SystemTime;
use tracing::info;

#[derive(Serialize, Deserialize, Clone)]
pub enum Operation {
    Get,
    Create,
    Update,
    Delete,
    Bootstrap,
    ApplyOwner(String),
    Unknown(String),
}

async fn apply_owner<T>(api: Api<T>, owner: String, value: &T) -> Result<(), Error>
where
    T: Clone + DeserializeOwned + Debug + Serialize + ResourceExt,
{
    let owner_ref: Value = json!({
        "metadata": {
            "ownerReferences": [{
                "apiVersion": "v1",
                "kind": "WebhookHelper",
                "name": "webhook-helper.io",
                "uid": owner,
                "blockOwnerDeletion": false
            }]
        }
    });

    let patch: Patch<&Value> = Patch::Merge(&owner_ref);
    api.patch(&value.name_any(), &PatchParams::default(), &patch)
        .await?;

    Ok(())
}

pub async fn perform_operation<T>(
    client: Client,
    operation: Operation,
    value: &T,
) -> Result<T, Error>
where
    T: Clone + DeserializeOwned + Debug + Serialize + ResourceExt,
    <T as kube::Resource>::DynamicType: Default,
    T: Resource<Scope = NamespaceResourceScope>,
{
    let pp = PostParams::default();
    let dp = DeleteParams::default();
    let api: Api<T> = Api::namespaced(client, &value.namespace().unwrap_or("default".into()));

    match operation {
        Operation::Create => {
            let result = api.create(&pp, value).await?;
            Ok(result)
        }
        Operation::Delete => {
            let _result = api.delete(&value.name_any(), &dp).await?;
            Ok(value.clone())
        }
        Operation::Get => {
            let result = api.get(&value.name_any()).await?;
            Ok(result)
        }
        Operation::Update => {
            let result = api.replace(&value.name_any(), &pp, value).await?;
            Ok(result)
        }
        Operation::Unknown(op) => Err(Error::UnknownOperation(op)),
        Operation::ApplyOwner(owner) => {
            apply_owner(api, owner, value).await?;
            Ok(value.clone())
        }
        Operation::Bootstrap => Err(Error::UnknownOperation("Bootstrap".into())),
    }
}

pub async fn perform_get<T>(client: Client, name: &str, namespace: &str) -> Result<T, kube::Error>
where
    T: Clone + DeserializeOwned + Debug + Serialize + ResourceExt,
    <T as kube::Resource>::DynamicType: Default,
    T: Resource<Scope = NamespaceResourceScope>,
{
    let api: Api<T> = Api::namespaced(client, namespace);
    let result = api.get(name).await?;
    Ok(result)
}

pub async fn perform_cluster_operation<T>(
    client: Client,
    operation: Operation,
    value: &T,
) -> Result<T, Error>
where
    T: Clone + DeserializeOwned + Debug + Serialize + ResourceExt,
    <T as kube::Resource>::DynamicType: Default,
    T: Resource<Scope = ClusterResourceScope>,
{
    let pp = PostParams::default();
    let dp = DeleteParams::default();
    let api: Api<T> = Api::all(client);

    match operation {
        Operation::Create => {
            let result = api.create(&pp, value).await?;
            Ok(result)
        }
        Operation::Delete => {
            let _result = api.delete(&value.name_any(), &dp).await?;
            Ok(value.clone())
        }
        Operation::Get => {
            let result = api.get(&value.name_any()).await?;
            Ok(result)
        }
        Operation::Update => {
            let result = api.replace(&value.name_any(), &pp, value).await?;
            Ok(result)
        }
        Operation::Unknown(op) => Err(Error::UnknownOperation(op)),
        Operation::ApplyOwner(owner) => {
            apply_owner(api, owner, value).await?;
            Ok(value.clone())
        }
        Operation::Bootstrap => Err(Error::UnknownOperation("Bootstrap".into())),
    }
}

pub async fn perform_cluster_get<T>(client: Client, name: &str) -> Result<T, kube::Error>
where
    T: Clone + DeserializeOwned + Debug + Serialize + ResourceExt,
    <T as kube::Resource>::DynamicType: Default,
    T: Resource<Scope = ClusterResourceScope>,
{
    let api: Api<T> = Api::all(client);
    let result = api.get(name).await?;
    Ok(result)
}

pub async fn update_status(
    client: Client,
    stage: Stage,
    resource: WebhookHelper,
) -> Result<WebhookHelper, Error> {
    info!("Updating Status");
    let pp = PostParams::default();
    let api: Api<WebhookHelper> = Api::all(client.clone());

    let mut result = api.get_status(&resource.name_any()).await?;

    let mut status = match result.status {
        Some(s) => s,
        None => WebhookHelperStatus::default(),
    };

    let datetime: DateTime<Utc> = SystemTime::now().into();
    let mut condition_entry = WebhookHelperCondition {
        type__: format!("{}", stage),
        message: stage.message(),
        status: "True".into(),
        last_transition_time: format!("{}", datetime.format("%d/%m/%Y %T")),
    };

    match stage {
        Stage::CreationFailed(_) => {
            condition_entry.status = "False".into();
        }
        Stage::DeploymentComplete(dep) | Stage::DeploymentStarted(dep) => {
            match dep {
                DeploymentType::Deployment(d) => {
                    status.deployment = Some(d.name_any());
                }
                DeploymentType::Pod(p) => {
                    status.pod = Some(p.name_any());
                }
            };
        }
        Stage::WebhookCreated(w) => {
            match w {
                WebhookType::Mutating(m) => {
                    status.mutating_webhook = Some(m.name_any());
                }
                WebhookType::Validating(v) => {
                    status.validating_webhook = Some(v.name_any());
                }
            };
        }
        Stage::CertificateCreated(c) => {
            status.certificate = Some(c);
        }
        Stage::ServiceCreated(s) => {
            status.service = Some(s.name_any());
        }
        _ => {}
    };

    if let Some(mut v) = status.conditions.clone() {
        v.push(condition_entry);
        status.conditions = Some(v);
    } else {
        status.conditions = Some(vec![condition_entry]);
    };

    result.status = Some(status);

    let data: Vec<u8> = serde_json::to_vec(&result)?;

    let updated_status = api.replace_status(&resource.name_any(), &pp, data).await?;
    // info!("Status Updated: {:?}", updated_status);
    Ok(updated_status)
}

pub async fn determine_stage(client: Client, value: WebhookHelper) -> Result<Stage, Error> {
    let api: Api<WebhookHelper> = Api::all(client.clone());
    let result = api.get_status(&value.name_any()).await?;
    let namespace = &value.spec.namespace;
    match result.status {
        Some(status) => {
            if let Some(conditions) = status.conditions {
                if let Some(last) = conditions.last() {
                    let result = match last.type__.as_str() {
                        "WebhookHelperCreated" => Ok(Stage::HelperCreated),
                        "CertificateCreated" => Ok(Stage::CertificateCreated(
                            status.certificate.unwrap_or("<unknown>".into()),
                        )),
                        "DeploymentComplete" => {
                            if let Some(deployment) = status.deployment {
                                let dep =
                                    perform_get(client.clone(), &deployment, namespace).await?;
                                Ok(Stage::DeploymentComplete(DeploymentType::Deployment(dep)))
                            } else if let Some(pod) = status.pod {
                                let pod = perform_get(client.clone(), &pod, namespace).await?;
                                Ok(Stage::DeploymentComplete(DeploymentType::Pod(pod)))
                            } else {
                                Err(Error::UnknownOperation(
                                    "Unable to determine deployment type".into(),
                                ))
                            }
                        }
                        "DeploymentStarted" => {
                            if let Some(deployment) = status.deployment {
                                let dep =
                                    perform_get(client.clone(), &deployment, namespace).await?;
                                Ok(Stage::DeploymentStarted(DeploymentType::Deployment(dep)))
                            } else if let Some(pod) = status.pod {
                                let pod = perform_get(client.clone(), &pod, namespace).await?;
                                Ok(Stage::DeploymentStarted(DeploymentType::Pod(pod)))
                            } else {
                                Err(Error::UnknownOperation(
                                    "Unable to determine deployment type".into(),
                                ))
                            }
                        }
                        "ServiceCreated" => {
                            if let Some(service) = status.service {
                                let result =
                                    perform_get(client.clone(), &service, namespace).await?;
                                Ok(Stage::ServiceCreated(result))
                            } else {
                                Err(Error::UnknownOperation("Unable to get service".into()))
                            }
                        }
                        "WebhookCreated" => {
                            if let Some(webhook) = status.mutating_webhook {
                                let mut_webhook =
                                    perform_cluster_get(client.clone(), &webhook).await?;
                                Ok(Stage::WebhookCreated(WebhookType::Mutating(mut_webhook)))
                            } else if let Some(webhook) = status.validating_webhook {
                                let val_webhook =
                                    perform_cluster_get(client.clone(), &webhook).await?;
                                Ok(Stage::WebhookCreated(WebhookType::Validating(val_webhook)))
                            } else {
                                Err(Error::UnknownOperation(
                                    "Unable to determine webhook type".into(),
                                ))
                            }
                        }
                        "CreationFailed" => Ok(Stage::CreationFailed(last.message.clone())),
                        _ => Err(Error::UnknownOperation(format!(
                            "Unable to determine condition type: {}",
                            last.type__
                        ))),
                    };
                    result
                } else {
                    Ok(Stage::HelperCreated)
                }
            } else {
                Ok(Stage::HelperCreated)
            }
        }
        None => Ok(Stage::HelperCreated),
    }
}
