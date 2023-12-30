use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::core::v1::PodSpec;
use k8s_openapi::api::apps::v1::Deployment;
use serde_json::Value;
use crate::controller::Error;

pub async fn validate_container_name(container_name: Option<String>, pod_spec: Option<PodSpec>) -> Result<(), Error> {

    if let Some(container_name) = container_name.clone() {
        let container_names: Vec<String> = match pod_spec.clone() {
            Some(pod_spec) => {
                pod_spec.containers.iter().filter(|p| p.name == container_name).map(|p| p.name.clone()).collect()
            },
            None => Vec::new(),
        };

        if container_names.len() != 1 {
            return Err(Error::UnableToCreateObject(format!("Container name {} not found in PodSpec", container_name)));
        };

    } else {
        let container_names: Vec<String> = match pod_spec.clone() {
            Some(pod_spec) => {
                pod_spec.containers.iter().map(|p| p.name.clone()).collect()
            },
            None => Vec::new(),
        };

        match container_names.len() {
            0 => return Err(Error::UnableToCreateObject("No Containers Specified in PodSpec".into())),
            1 => {},
            _ => return Err(Error::UnableToCreateObject("Too many containers in PodSpec, specify ContainerName in WebhookHelper Spec".into())),
        };
    };

    Ok(())
}

pub async fn convert_to_deployment(data: Value) -> Result<Deployment, Error> {
    let value: Deployment = serde_json::from_value(data)?;
    Ok(value)
}

pub async fn convert_to_pod(data: Value) -> Result<Pod, Error> {
    let value: Pod = serde_json::from_value(data)?;
    Ok(value)
}