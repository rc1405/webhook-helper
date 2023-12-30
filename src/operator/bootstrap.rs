use kube::Client;
use kube::core::ResourceExt;
use tracing::info;
use crate::crd::WebhookHelper;
use crate::controller::Error;
use crate::operator::Operation;
use super::certificate::CertificateStage;
use super::backend::DeploymentStage;
use super::service::ServiceStage;
use super::webhook::WebhookStage;


pub async fn bootstrap(client: Client, webhook: WebhookHelper) -> Result<(), Error> {

    let mut cert = CertificateStage::new(client.clone(), Operation::Bootstrap, webhook.clone());
    cert.run().await?;
    
    let mut deployment = DeploymentStage::new(
        client.clone(), 
        Operation::Bootstrap, 
        webhook.clone(),
        Some(cert.get_secret().await.unwrap().name_any())
    );

    deployment.run().await?;

    let mut service = ServiceStage::new(
        client.clone(), 
        Operation::Bootstrap, 
        webhook.clone(), 
        deployment.get_deployment().await
    );
    service.run().await?;
    
    let mut webhook_stage = WebhookStage::new(
        client.clone(), 
        Operation::Bootstrap, 
        webhook.clone(), 
        service.get_service().await
    );
    webhook_stage.run().await?;
    info!("Bootstrap Complete!");

    Ok(())
}