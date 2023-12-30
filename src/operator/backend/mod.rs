use kube::Client;
use kube::core::ResourceExt;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::apps::v1::Deployment;

use crate::crd::{WebhookHelper, DeploymentType, Stage};
use crate::controller::Error;
use super::{Operation, update_status, determine_stage, perform_get};
use super::perform_operation;

mod utils;
mod pod;
mod deployment;
pub use utils::{validate_container_name, convert_to_deployment, convert_to_pod};

pub struct DeploymentStage {
    client: Client,
    operation: Operation,
    webhook: WebhookHelper,
    secret: Option<String>,
    deployment: Option<DeploymentType>,
}

impl DeploymentStage {
    pub fn new(client: Client, operation: Operation, webhook: WebhookHelper, secret: Option<String>) -> DeploymentStage {
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
                                },
                                _ => return Err(e),
                            };
                        },
                    };
                };

                return Ok(())
            },
            Operation::Delete => {
                if let Some(status) = self.webhook.status.clone() {
                    if let Some(dep) = status.deployment {
                        let deployment: Deployment = perform_get(self.client.clone(), &dep, &self.webhook.spec.namespace).await?;
                        self.deployment = Some(DeploymentType::Deployment(deployment));
                    } else if let Some(p) = status.pod {
                        let pod: Pod = perform_get(self.client.clone(), &p, &self.webhook.spec.namespace).await?;
                        self.deployment = Some(DeploymentType::Pod(pod));
                    } else {
                        return Ok(())
                    };
                    self.delete().await?;
                };
                return Ok(())
            },
            _ => {
                let stage = determine_stage(self.client.clone(), self.webhook.clone()).await?;
                match stage {
                    Stage::CertificateCreated(_) => {
                        let deployment = self.create_deployment().await?;
                        update_status(
                            self.client.clone(), 
                            Stage::DeploymentStarted(deployment.clone()), 
                            self.webhook.clone(),
                        ).await?;
                        if let Some(uid) = self.webhook.uid() {
                            match deployment {
                                DeploymentType::Deployment(d) => {
                                    perform_operation(self.client.clone(), Operation::ApplyOwner(uid), &d).await?;
                                },
                                DeploymentType::Pod(p) => {
                                    perform_operation(self.client.clone(), Operation::ApplyOwner(uid), &p).await?;
                                },
                            };
                        };

                    },
                    Stage::DeploymentStarted(deployment) => {
                        self.deployment = Some(deployment);
                        let result = self.get_deployment_status().await?;
                        update_status(
                            self.client.clone(), 
                            Stage::DeploymentComplete(result), 
                            self.webhook.clone(),
                        ).await?;
                    },
                    _ => {
                        // check if operation is update
                    },
                };
        
            },
        };
        Ok(())
    }
}