use kube::core::ObjectMeta;
use kube::core::ResourceExt;
use std::collections::BTreeMap;
use tracing::{error, info};

use super::perform_operation;
use super::DeploymentStage;
use super::Operation;
use super::{convert_to_deployment, convert_to_pod, validate_container_name};
use crate::controller::Error;
use crate::crd::DeploymentType;

impl DeploymentStage {
    pub async fn create_deployment(&mut self) -> Result<DeploymentType, Error> {
        if let Ok(mut d) = convert_to_deployment(self.webhook.spec.deployment.clone()).await {
            d.metadata.namespace = Some(self.webhook.spec.namespace.clone());

            self.add_labels(&mut d).await;

            match d.spec.clone() {
                Some(mut deployment_spec) => {
                    validate_container_name(
                        self.webhook.spec.container_name.clone(),
                        deployment_spec.template.spec.clone(),
                    )
                    .await?;
                    if let Some(mut pod_meta) = deployment_spec.template.metadata.clone() {
                        if let Some(mut labels) = pod_meta.labels.clone() {
                            labels.insert(
                                "app.kubernetes.io/managed-by".into(),
                                "webhook-helper".into(),
                            );
                            pod_meta.labels = Some(labels);
                        } else {
                            let mut labels: BTreeMap<String, String> = BTreeMap::new();
                            labels.insert(
                                "app.kubernetes.io/managed-by".into(),
                                "webhook-helper".into(),
                            );
                            pod_meta.labels = Some(labels);
                        };
                        deployment_spec.template.metadata = Some(pod_meta);
                    } else {
                        let mut labels: BTreeMap<String, String> = BTreeMap::new();
                        labels.insert(
                            "app.kubernetes.io/managed-by".into(),
                            "webhook-helper".into(),
                        );
                        deployment_spec.template.metadata = Some(ObjectMeta {
                            labels: Some(labels),
                            ..Default::default()
                        });
                    };

                    if let Some(mut pod_spec) = deployment_spec.template.spec.clone() {
                        self.add_volume_mount(&mut pod_spec).await?;
                        deployment_spec.template.spec = Some(pod_spec);
                    };

                    d.spec = Some(deployment_spec);
                }
                None => {
                    return Err(Error::UnableToCreateObject(
                        "No DeploymentSpec found in WebhookHelper".into(),
                    ))
                }
            };

            let result = perform_operation(self.client.clone(), Operation::Create, &d).await?;
            let deployment_type = DeploymentType::Deployment(result);
            self.deployment = Some(deployment_type.clone());
            info!("Deployment {} started", d.name_any());
            Ok(deployment_type)
        } else if let Ok(mut p) = convert_to_pod(self.webhook.spec.deployment.clone()).await {
            p.metadata.namespace = Some(self.webhook.spec.namespace.clone());
            self.add_labels(&mut p).await;

            validate_container_name(self.webhook.spec.container_name.clone(), p.spec.clone())
                .await?;

            if let Some(mut pod_spec) = p.spec.clone() {
                self.add_volume_mount(&mut pod_spec).await?;
                p.spec = Some(pod_spec);
            };

            let result = perform_operation(self.client.clone(), Operation::Create, &p).await?;
            let deployment_type = DeploymentType::Pod(result);
            self.deployment = Some(deployment_type.clone());
            info!("Deployment {} started", p.name_any());
            Ok(deployment_type)
        } else {
            error!("Unable to determine kind of deployment object");
            return Err(Error::UnableToCreateObject(
                "Invalid deployment spec".into(),
            ));
        }
    }

    pub async fn get_deployment_status(&self) -> Result<DeploymentType, Error> {
        if let Some(deployment) = self.deployment.clone() {
            match deployment {
                DeploymentType::Deployment(dep) => {
                    let result =
                        perform_operation(self.client.clone(), Operation::Get, &dep).await?;
                    if let Some(status) = result.clone().status {
                        if let Some(ready_replicas) = status.ready_replicas {
                            if ready_replicas > 0 {
                                info!("Deployment {} is ready", dep.name_any());
                                return Ok(DeploymentType::Deployment(result.clone()));
                            };
                        };
                    };
                }
                DeploymentType::Pod(pod) => {
                    let result =
                        perform_operation(self.client.clone(), Operation::Get, &pod).await?;
                    if let Some(status) = result.status.clone() {
                        if let Some(container_status) = status.conditions {
                            let statuses: Vec<String> = container_status
                                .iter()
                                .filter(|s| s.type_ == "Ready")
                                .map(|s| s.status.clone())
                                .collect();
                            if statuses
                                .iter()
                                .filter(|n| *n == &"True".to_string())
                                .count()
                                > 0
                            {
                                info!("Deployment {} is ready", pod.name_any());
                                return Ok(DeploymentType::Pod(result.clone()));
                            };
                        };
                    };
                }
            };
        };
        Err(Error::ResourceNotReady)
    }

    pub async fn get_deployment(&self) -> Option<DeploymentType> {
        self.deployment.clone()
    }

    pub async fn delete(&self) -> Result<(), Error> {
        if let Some(deployment) = self.deployment.clone() {
            match deployment {
                DeploymentType::Deployment(dep) => {
                    perform_operation(self.client.clone(), Operation::Delete, &dep).await?;
                }
                DeploymentType::Pod(pod) => {
                    perform_operation(self.client.clone(), Operation::Delete, &pod).await?;
                }
            };
        };
        Ok(())
    }
}
