use kube::api::{Patch, PatchParams};
use kube::core::ResourceExt;
use kube::runtime::controller::Action;
use kube::{Api, Client};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{error, info, warn};

use futures::StreamExt;
use kube::runtime::{controller::Controller, watcher, Config};

use crate::crd::{Stage, WebhookHelper};
use crate::operator::{
    determine_stage, CertificateStage, DeploymentStage, Operation, ServiceStage, WebhookStage,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to create Webhook: {0}")]
    WebhookHelperCreationFailed(#[from] kube::Error),
    #[allow(dead_code)]
    #[error("UnableToCreateObject: {0}")]
    UnableToCreateObject(String),
    #[error("UnableToSerializeObject: {0}")]
    UnableToSerializeObject(#[from] serde_json::Error),
    #[error("UnableToApproveCertificate: {0}")]
    UnableToApproveCertificate(#[from] http::Error),
    #[error("UnableToCreateCertificate: {0}")]
    UnableToCreateCertificate(#[from] rcgen::RcgenError),
    #[error("UnknownOperation: {0}")]
    UnknownOperation(String),
    #[error("ResourceNotReady")]
    ResourceNotReady,
    #[error("UnableToDetermineContainerName")]
    UnableToDetermineContainerName,
}

struct CustomClients {
    kube: Client,
}

enum CustomAction {
    /// Create the subresources, this includes spawning `n` pods with Echo service
    Create,
    /// Delete all subresources created in the `Create` phase
    Delete,
    Update,
    /// This `Echo` resource is in desired state and requires no actions to be taken
    NoOp,
}

/// Resources arrives into reconciliation queue in a certain state. This function looks at
/// the state of given `Echo` resource and decides which actions needs to be performed.
/// The finite set of possible actions is represented by the `EchoAction` enum.
///
/// # Arguments
/// - `echo`: A reference to `Echo` being reconciled to decide next action upon.
fn determine_action<T: kube::core::Resource>(echo: &T) -> CustomAction {
    return if echo.meta().deletion_timestamp.is_some() {
        echo.meta()
            .finalizers
            .as_ref()
            .map_or(CustomAction::NoOp, |finalizers| {
                if !finalizers.is_empty() {
                    for finalizer in finalizers {
                        if finalizer.starts_with("webhook-helper.io") {
                            return CustomAction::Delete;
                        };
                    }
                    return CustomAction::NoOp;
                };
                CustomAction::NoOp
            })
    } else {
        echo.meta()
            .finalizers
            .as_ref()
            .map_or(CustomAction::Create, |finalizers| {
                if !finalizers.is_empty() {
                    for finalizer in finalizers {
                        if finalizer.starts_with("webhook-helper.io") {
                            return CustomAction::Update;
                        };
                    }
                    return CustomAction::Create;
                };
                CustomAction::Create
            })
    };
}

/// The reconciler that will be called when either object change
async fn reconcile(g: Arc<WebhookHelper>, ctx: Arc<CustomClients>) -> Result<Action, Error> {
    // .. use api here to reconcile a child ConfigMap with ownerreferences
    // see configmapgen_controller example for full info
    let webhook_api: Api<WebhookHelper> = Api::all(ctx.kube.clone());
    let name = g.name_any();

    let d = match webhook_api.get(name.as_str()).await {
        Ok(def) => Some(def),
        Err(e) => {
            error!("Error getting item: {:?}", e);
            match e {
                kube::Error::Api(error_response) => {
                    if error_response.code == 404 {
                        None
                    } else {
                        return Ok(Action::requeue(Duration::from_secs(15)));
                    }
                }
                _ => return Ok(Action::requeue(Duration::from_secs(30))),
            }
        }
    };

    if let Some(z) = d {
        match determine_action(&z) {
            CustomAction::Create => {
                info!("Creating webhook {}", z.name_any());
                let mut cert_stage =
                    CertificateStage::new(ctx.kube.clone(), Operation::Create, z.clone());
                match cert_stage.run().await {
                    Ok(_) => {
                        let finalizer: Value = json!({
                            "metadata": {
                                "finalizers": ["webhook-helper.io"]
                            }
                        });
                        let patch: Patch<&Value> = Patch::Merge(&finalizer);
                        webhook_api
                            .patch(&name, &PatchParams::default(), &patch)
                            .await?;
                        return Ok(Action::requeue(Duration::from_secs(5)));
                    }
                    Err(e) => return Err(e),
                };
            }
            CustomAction::Delete => {
                info!("Deleting webhook {}", z.name_any());

                if let Some(status) = z.status.clone() {
                    if status.pod.is_some() || status.deployment.is_some() {
                        info!("Deleting deployment for {}", z.name_any());
                        let mut deploy_stage = DeploymentStage::new(
                            ctx.kube.clone(),
                            Operation::Delete,
                            z.clone(),
                            None,
                        );
                        deploy_stage.run().await?;
                    };

                    if status.validating_webhook.is_some() || status.mutating_webhook.is_some() {
                        info!("Deleting webhook {}", z.name_any());
                        let mut webhook_stage =
                            WebhookStage::new(ctx.kube.clone(), Operation::Delete, z.clone(), None);
                        webhook_stage.run().await?;
                    };

                    if status.service.is_some() {
                        info!("Deleting service {}", z.name_any());
                        let mut service_stage =
                            ServiceStage::new(ctx.kube.clone(), Operation::Delete, z.clone(), None);
                        service_stage.run().await?;
                    };

                    if status.certificate.is_some() {
                        let mut cert_stage =
                            CertificateStage::new(ctx.kube.clone(), Operation::Delete, z.clone());
                        cert_stage.run().await?;
                    }
                };

                let finalizer: Value = json!({
                    "metadata": {
                        "finalizers": null
                    }
                });
                let patch: Patch<&Value> = Patch::Merge(&finalizer);

                webhook_api
                    .patch(&name, &PatchParams::default(), &patch)
                    .await?;

                return Ok(Action::await_change());
            }
            CustomAction::Update => match determine_stage(ctx.kube.clone(), z.clone()).await? {
                Stage::HelperCreated => {
                    info!("Helper status found");
                }
                Stage::CertificateCreated(s) => {
                    info!("Creating deployment for {}", z.name_any());
                    let mut deploy_stage = DeploymentStage::new(
                        ctx.kube.clone(),
                        Operation::Create,
                        z.clone(),
                        Some(s),
                    );
                    deploy_stage.run().await?;
                    return Ok(Action::requeue(Duration::from_secs(10)));
                }
                Stage::CreationFailed(_) => {
                    info!("Creation failed for {}", z.name_any());
                    return Ok(Action::await_change());
                }
                Stage::DeploymentComplete(d) => {
                    info!("Deployment is complete for {}", z.name_any());
                    let mut service_stage =
                        ServiceStage::new(ctx.kube.clone(), Operation::Create, z.clone(), Some(d));
                    service_stage.run().await?;
                    return Ok(Action::requeue(Duration::from_secs(10)));
                }
                Stage::DeploymentStarted(_d) => {
                    info!("Checking deployment status for {}", z.name_any());
                    let mut deploy_stage =
                        DeploymentStage::new(ctx.kube.clone(), Operation::Create, z.clone(), None);
                    match deploy_stage.run().await {
                        Ok(_) => return Ok(Action::requeue(Duration::from_secs(10))),
                        Err(e) => match e {
                            Error::ResourceNotReady => {
                                return Ok(Action::requeue(Duration::from_secs(10)))
                            }
                            _ => return Err(e),
                        },
                    };
                }
                Stage::ServiceCreated(s) => {
                    info!("Creating webhook {}", z.name_any());
                    let mut webhook_stage =
                        WebhookStage::new(ctx.kube.clone(), Operation::Create, z.clone(), Some(s));
                    webhook_stage.run().await?;
                    return Ok(Action::requeue(Duration::from_secs(10)));
                }
                Stage::WebhookCreated(_) => {
                    info!("Webhook created {}", z.name_any());
                    return Ok(Action::await_change());
                }
                Stage::Deleting => {}
            },
            CustomAction::NoOp => return Ok(Action::await_change()),
        }
    } else {
        println!("Item was none! {:?}\n", g);
    };

    Ok(Action::requeue(Duration::from_secs(5)))
}
/// an error handler that will be called when the reconciler fails with access to both the
/// object that caused the failure and the actual error
fn error_policy(obj: Arc<WebhookHelper>, error: &Error, _ctx: Arc<CustomClients>) -> Action {
    let def_str = serde_json::to_string(&obj.spec).unwrap();
    println!("{} received error {:?}", def_str, error);
    Action::requeue(Duration::from_secs(60))
}

pub async fn run() -> Result<(), Error> {
    let client = Client::try_default().await?;
    let api: Api<WebhookHelper> = Api::all(client.clone());

    let clients = CustomClients {
        kube: client.clone(),
    };

    let context = Arc::new(clients); // bad empty context - put client in here

    let config = Config::default().concurrency(2);

    Controller::new(api.clone(), watcher::Config::default())
        .owns(api, watcher::Config::default())
        .with_config(config.clone())
        .shutdown_on_signal()
        .run(reconcile, error_policy, context.clone())
        .for_each(|res| async move {
            match res {
                Ok((o, a)) => {
                    let message = format!("reconcile {} complete for {:?}", o.name, a);
                    info!(message);
                }
                Err(e) => match e {
                    kube::runtime::controller::Error::QueueError(queue_error) => {
                        match queue_error {
                            watcher::Error::WatchError(watch_error) => {
                                if watch_error.code != 410 && watch_error.reason != *"Expired" {
                                    warn!("reconcile failed: {:?}", watch_error)
                                };
                            }
                            _ => warn!("reconcile failed: {:?}", queue_error),
                        }
                    }
                    _ => warn!("reconcile failed: {:?}", e),
                },
            }
        })
        .await;

    println!("Controller terminated");

    Ok(())
}
