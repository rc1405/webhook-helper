use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::core::ResourceExt;
use kube::Client;

use super::perform_operation;
use super::{determine_stage, perform_get, update_status, Operation};
use crate::controller::Error;
use crate::crd::{DeploymentType, Stage, WebhookHelper};

mod deployment;
mod pod;
mod utils;
pub use utils::{convert_to_deployment, convert_to_pod, validate_container_name};

pub struct DeploymentStage {
    client: Client,
    operation: Operation,
    webhook: WebhookHelper,
    secret: Option<String>,
    deployment: Option<DeploymentType>,
}

impl DeploymentStage {
    pub fn new(
        client: Client,
        operation: Operation,
        webhook: WebhookHelper,
        secret: Option<String>,
    ) -> DeploymentStage {
        DeploymentStage {
            client,
            operation,
            webhook,
            deployment: None,
            secret,
        }
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        match self.operation {
            Operation::Bootstrap => {
                self.create_deployment().await?;

                loop {
                    match self.get_deployment_status().await {
                        Ok(_) => break,
                        Err(e) => {
                            match e {
                                Error::ResourceNotReady => {
                                    std::thread::sleep(std::time::Duration::from_secs(10));
                                }
                                _ => return Err(e),
                            };
                        }
                    };
                }

                return Ok(());
            }
            Operation::Delete => {
                if let Some(status) = self.webhook.status.clone() {
                    if let Some(dep) = status.deployment {
                        let deployment: Deployment =
                            perform_get(self.client.clone(), &dep, &self.webhook.spec.namespace)
                                .await?;
                        self.deployment = Some(DeploymentType::Deployment(deployment));
                    } else if let Some(p) = status.pod {
                        let pod: Pod =
                            perform_get(self.client.clone(), &p, &self.webhook.spec.namespace)
                                .await?;
                        self.deployment = Some(DeploymentType::Pod(pod));
                    } else {
                        return Ok(());
                    };
                    self.delete().await?;
                };
                return Ok(());
            }
            _ => {
                let stage = determine_stage(self.client.clone(), self.webhook.clone()).await?;
                match stage {
                    Stage::CertificateCreated(_) => {
                        let deployment = self.create_deployment().await?;
                        update_status(
                            self.client.clone(),
                            Stage::DeploymentStarted(deployment.clone()),
                            self.webhook.clone(),
                        )
                        .await?;
                        if let Some(uid) = self.webhook.uid() {
                            match deployment {
                                DeploymentType::Deployment(d) => {
                                    perform_operation(
                                        self.client.clone(),
                                        Operation::ApplyOwner(uid),
                                        &d,
                                    )
                                    .await?;
                                }
                                DeploymentType::Pod(p) => {
                                    perform_operation(
                                        self.client.clone(),
                                        Operation::ApplyOwner(uid),
                                        &p,
                                    )
                                    .await?;
                                }
                            };
                        };
                    }
                    Stage::DeploymentStarted(deployment) => {
                        self.deployment = Some(deployment);
                        let result = self.get_deployment_status().await?;
                        update_status(
                            self.client.clone(),
                            Stage::DeploymentComplete(result),
                            self.webhook.clone(),
                        )
                        .await?;
                    }
                    _ => {
                        // check if operation is update
                    }
                };
            }
        };
        Ok(())
    }
}
