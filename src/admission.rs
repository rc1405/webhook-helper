use kube::core::{
    admission::{AdmissionRequest, AdmissionResponse, AdmissionReview},
    DynamicObject,
};
use tracing::info;
use std::convert::{From, Infallible};
use kube::Client;
use serde_json::Value;
use warp::{reply, Filter, Reply};
use k8s_openapi::api::{core::v1::Pod, apps::v1::Deployment, admissionregistration::v1::{ValidatingWebhook, MutatingWebhook}};


use crate::crd::WebhookHelper;
use crate::controller::Error;
use crate::operator::validate_container_name;

pub async fn serve(port: u16) -> Result<(), Error> {
    let client = Client::try_default().await?;

    let routes = warp::path("validate")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(move | body: AdmissionReview<DynamicObject> | {
            handler(client.clone(), body)
        })   
        .with(warp::trace::request());

    warp::serve(warp::post().and(routes))
        .tls()
        .cert_path("/webhook-helper/tls.crt")
        .key_path("/webhook-helper/tls.key")
        .run(([0, 0, 0, 0], port)) 
        .await;
    
    Ok(())
}

#[allow(unused_variables)]
async fn handler(client: Client, body: AdmissionReview<DynamicObject>) -> Result<impl Reply, Infallible> {
    // Parse incoming webhook AdmissionRequest first
    let req: AdmissionRequest<_> = match body.try_into() {
        Ok(req) => req,
        Err(err) => {
            return Ok(reply::json(
                &AdmissionResponse::invalid(err.to_string()).into_review(),
            ));
        }
    };

    let mut res = AdmissionResponse::from(&req);
    let raw: Value = match req.object {
        Some(o) => {
            if let Ok(r) = serde_json::to_value(o) {
                r
            } else {
                res = res.deny("invalid request format".to_string().to_string());
                return Ok(reply::json(&res.into_review()))
            }
        },
        None => return Ok(reply::json(&res.into_review())),
    };

    let resource: WebhookHelper = match serde_json::from_value(raw) {
        Ok(v) => v,
        Err(_) => {
            res = res.deny("invalid request format".to_string().to_string());
            return Ok(reply::json(&res.into_review()))
        },
    };

    let deployment: Option<Deployment> = match serde_json::from_value(resource.spec.deployment.clone()) {
        Ok(v) => Some(v),
        Err(_) => None,
    };

    if let Some(d) = deployment.clone() {
        match d.spec.clone() {
            Some(deployment_spec) => {
                if let Err(e) = validate_container_name(resource.spec.container_name.clone(), deployment_spec.template.spec.clone()).await {
                    res = res.deny(format!("{}", e));
                    return Ok(reply::json(&res.into_review()))
                }
            },
            None => {
                res = res.deny(format!("{}", Error::UnableToCreateObject("No DeploymentSpec found in WebhookHelper".into())));
                return Ok(reply::json(&res.into_review()))
            },
        }
    };

    let pod: Option<Pod> = match serde_json::from_value(resource.spec.deployment) {
        Ok(p) => Some(p),
        Err(_) => None,
    };

    if let Some(p) = pod.clone() {
        if let Err(e) = validate_container_name(resource.spec.container_name.clone(), p.spec.clone()).await {
            res = res.deny(format!("{}", e));
            return Ok(reply::json(&res.into_review()))
        };
    };

    if deployment.is_none() && pod.is_none() {
        res = res.deny("invalid request format".to_string().to_string());
        return Ok(reply::json(&res.into_review()))
    };

    let validating_webhook: Option<ValidatingWebhook> = match serde_json::from_value(resource.spec.webhook.clone()) {
        Ok(v) => Some(v),
        Err(_) => None,
    };

    let mutating_webhook: Option<MutatingWebhook> = match serde_json::from_value(resource.spec.webhook) {
        Ok(v) => Some(v),
        Err(_) => None,
    };

    if validating_webhook.is_none() && mutating_webhook.is_none() {
        res = res.deny("invalid request format".to_string().to_string());
            return Ok(reply::json(&res.into_review()))
    };

    info!("Webhook helper validated");

    // Wrap the AdmissionResponse wrapped in an AdmissionReview
    Ok(reply::json(&res.into_review()))
}