use kube::core::ResourceExt;
use kube::core::ObjectMeta;
use k8s_openapi::api::core::v1::{ServiceSpec, ServicePort};
use kube::Client;
use k8s_openapi::api::core::v1::Service;
use std::collections::BTreeMap;

use crate::controller::Error;
use super::perform_get;
use super::{Operation, update_status, determine_stage};
use crate::crd::{WebhookHelper, DeploymentType, Stage};
use super::perform_operation;

pub struct ServiceStage {
    client: Client,
    operation: Operation,
    webhook: WebhookHelper,
    deployment: Option<DeploymentType>,
    service: Option<Service>
}

impl ServiceStage {
    pub fn new(client: Client, operation: Operation, webhook: WebhookHelper, deployment: Option<DeploymentType>) -> ServiceStage {
        ServiceStage { 
            client, 
            operation,
            webhook,
            deployment,
            service: None,
        }
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        match self.operation {
            Operation::Bootstrap => {
                if self.deployment.is_none() {
                    return Err(Error::UnknownOperation("Deployment is not known".into()))
                };
                self.create_service().await?;
                return Ok(())
            },
            Operation::Delete => {
                if let Some(status) = self.webhook.status.clone() {
                    if let Some(s) = status.service {
                        let service: Service = perform_get(self.client.clone(), &s, &self.webhook.spec.namespace).await?;
                        self.service = Some(service);
                        self.delete().await?;
                    };
                };
                return Ok(())
            },
            _ => {
                let stage = determine_stage(self.client.clone(), self.webhook.clone()).await?;
                match stage {
                    Stage::DeploymentComplete(deployment) => {
                        self.deployment = Some(deployment);
                        let service = self.create_service().await?;
                        update_status(
                            self.client.clone(), 
                            Stage::ServiceCreated(service), 
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

    async fn create_service(&mut self) -> Result<Service, Error> {   
        let name = match self.deployment.clone().unwrap() {
            DeploymentType::Deployment(d) => d.name_any(),
            DeploymentType::Pod(p) => p.name_any(),
        };

        let mut selector_map: BTreeMap<String, String> = BTreeMap::new();
        selector_map.insert("app".to_string(), name.clone());
        selector_map.insert("app.kubernetes.io/managed-by".into(),  "webhook-helper".into());
    
        let service = Service{
            metadata: ObjectMeta {
                name: Some(name),
                namespace: Some(self.webhook.spec.namespace.clone()),
                ..Default::default()
            },
            spec: Some(ServiceSpec{
                selector: Some(selector_map),
                ports: Some(vec![
                    ServicePort{
                        protocol: Some("TCP".into()),
                        port: self.webhook.spec.listening_port,
                        ..Default::default()
                    }
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = perform_operation(self.client.clone(), Operation::Create, &service).await?;
        self.service = Some(result.clone());
        Ok(result)
    }

    pub async fn get_service(&self) -> Option<Service> {
        self.service.clone()
    }

    pub async fn delete(&self) -> Result<(), Error> {
        if let Some(service) = self.service.clone() {
            perform_operation(self.client.clone(), Operation::Delete, &service).await?;
        };
        Ok(())
    }
}

