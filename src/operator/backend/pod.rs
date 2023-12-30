use k8s_openapi::api::core::v1::VolumeMount;
use kube::core::ResourceExt;
use k8s_openapi::api::core::v1::PodSpec;
use k8s_openapi::api::core::v1::Volume;
use k8s_openapi::api::core::v1::SecretVolumeSource;
use k8s_openapi::api::core::v1::Container;
use std::collections::BTreeMap;
use crate::controller::Error;
use super::DeploymentStage;

impl DeploymentStage {
    pub async fn add_volume_mount(&self, pod_spec: &mut PodSpec) -> Result<(), Error> {
        let container_name = match self.webhook.spec.container_name.clone() {
            Some(s) => s,
            None => {
                if let Some(c) = pod_spec.containers.first() {
                    c.name.clone()
                } else {
                    return Err(Error::UnableToDetermineContainerName)
                }
            }
        };

        let mut containers: Vec<Container> = Vec::new();

        for mut c in pod_spec.containers.clone() {
            if c.name == container_name {
                if let Some(mut volumes) = c.volume_mounts.clone() {
                    volumes.push(VolumeMount{
                        mount_path: "/webhook-helper".into(),
                        name: "webhook-helper".into(),
                        read_only: Some(true),
                        ..Default::default()
                    });
                    c.volume_mounts = Some(volumes);
                } else {
                    c.volume_mounts = Some(vec![VolumeMount{
                        mount_path: "/webhook-helper".into(),
                        name: "webhook-helper".into(),
                        read_only: Some(true),
                        ..Default::default()
                    }]);
                };
            };
            containers.push(c);
        };
        pod_spec.containers = containers;

        if let Some(secret) = self.secret.clone() {
            if let Some(mut volumes) = pod_spec.volumes.clone() {
                volumes.push(Volume{
                    name: "webhook-helper".into(),
                    secret: Some(SecretVolumeSource { 
                        secret_name: Some(secret.clone()),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
                pod_spec.volumes = Some(volumes);
            } else {
                pod_spec.volumes = Some(vec![
                    Volume{
                        name: "webhook-helper".into(),
                        secret: Some(SecretVolumeSource { 
                            secret_name: Some(secret.clone()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }
                ])
            }
        };

        Ok(())
    }

    pub async fn add_labels<T: ResourceExt>(&self, resource: &mut T) {
        if let Some(mut labels) = resource.meta().labels.clone() {
            labels.insert("app.kubernetes.io/managed-by".into(),  "webhook-helper".into());
            resource.meta_mut().labels = Some(labels);
        } else {
            let mut labels: BTreeMap<String, String> = BTreeMap::new();
            labels.insert("app.kubernetes.io/managed-by".into(),  "webhook-helper".into());
            resource.meta_mut().labels = Some(labels);
        };
    }
}