#![allow(unused_imports, unused_variables)]

use crate::{Error, Result};
use actix_web::guard::Post;
use futures::StreamExt;

use jiff::{Timestamp, tz::Dst::No};
use k8s_openapi::{api::{apps::v1::{Deployment, DeploymentSpec}, core::v1::{Capabilities, Container, ContainerPort, PodSecurityContext, PodSpec, PodTemplateSpec, SeccompProfile, SecurityContext}}, apimachinery::pkg::apis::meta::v1::{Condition, LabelSelector, Time}};
use std::{collections::BTreeMap, sync::Arc};
use tokio::{sync::RwLock, time::Duration};
use serde::{Deserialize, Serialize};
use kube::{
    Api, Client, CustomResource, CustomResourceExt, Resource, ResourceExt, api::{ListParams, ObjectMeta, Patch, PatchParams, PostParams}, runtime::{
        controller::{Action, Controller}, events::{Recorder, Reporter}, watcher,
    },
};

use schemars::JsonSchema;


/// Diagnostics to be exposed by the web server
#[derive(Clone, Serialize)]
pub struct Diagnostics {
    #[serde(deserialize_with = "from_ts")]
    pub last_event: Timestamp,
    #[serde(skip)]
    pub reporter: Reporter,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self {
            last_event: Timestamp::now(),
            reporter: "doc-controller".into(),
        }
    }
}

impl Diagnostics {
    fn recorder(&self, client: Client) -> Recorder {
        Recorder::new(client, self.reporter.clone())
    }
}
#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[cfg_attr(test, derive(Default))]
#[kube(kind = "Memcached", group = "kube.rs", version = "v1", namespaced)]
#[kube(status = "MemcachedStatus")]
pub struct MemcachedSpec {
    pub size: i32
}

#[derive(Deserialize, Serialize, Clone, Default, Debug, JsonSchema)]
pub struct MemcachedStatus {
    pub conditions: Vec<Condition>,
}

/// State shared between the controller and the web server
#[derive(Clone, Default)]
pub struct State {
    /// Diagnostics populated by the reconciler
    diagnostics: Arc<RwLock<Diagnostics>>,
}

impl State {
    pub async fn diagnostics(&self) -> Diagnostics {
        self.diagnostics.read().await.clone()
    }

    pub async fn to_context(&self, client: Client) -> Arc<Context> {
        Arc::new(Context {
            client: client.clone(),
            recorder: self.diagnostics.read().await.recorder(client),
            diagnostics: self.diagnostics.clone(),
        })
    }
}

// Context for our reconciler
#[derive(Clone)]
pub struct Context {
    /// Kubernetes client
    pub client: Client,
    /// Event recorder
    pub recorder: Recorder,
    /// Diagnostics read by the web server
    pub diagnostics: Arc<RwLock<Diagnostics>>,
}
async fn reconcile(memcached: Arc<Memcached>, ctx: Arc<Context>) -> Result<Action> {
    println!("RECONCILING !");

    let ns = memcached.namespace().unwrap();
    let api: Api<Memcached> = Api::namespaced(ctx.client.clone(), &ns);

   	// Let's just set the status as Unknown when no status is available

    let conditions: &[Condition] = match &memcached.status {
        Some(status) => &status.conditions,
        None => &[],
    };

    if conditions.is_empty() {
        let status = MemcachedStatus {
            conditions: vec![Condition {
                type_: "Available".into(),
                status: "Unknown".into(),
                reason: "Reconciling".into(),
                message: "Starting reconciliation".into(),
                last_transition_time: Time(Timestamp::now()),
                observed_generation: memcached.metadata.generation,
            }],
        };
        let patch = serde_json::json!({ "status": status });
        api.patch_status(memcached.metadata.name.as_deref().unwrap(), &PatchParams::default(), &Patch::Merge(&patch))
            .await
            .map_err(Error::KubeError)?;
    }
    
   	// Check if the deployment already exists, if not create a new one

    let deployments: Api<Deployment> = Api::namespaced(ctx.client.clone(), &ns);
    match deployments.get_opt(&memcached.name_any()).await.map_err(Error::KubeError)? {
        None => {
            println!("pas d'opérateur trouvé");
            println!("création de l'operateur trouvé");
            match deployments.create(&PostParams::default(), &deployment_for_memcached(&memcached)).await {
                Ok(dep) => {
                    // Deployment created successfully
            		// We will requeue the reconciliation so that we can ensure the state
            		// and move forward for the next operations
                    return Ok(Action::requeue(Duration::from_secs(60)))
                }
                Err(e) => {
                    println!("Failed to define new Deployment resource for Memcached");
                    let status = MemcachedStatus {
                        conditions: vec![Condition {
                            type_: "Available".into(),
                            status: "False".into(),
                            reason: "Reconciling".into(),
                            message: "Failed to create Deployment for the custom resource".into(),
                            last_transition_time: Time(Timestamp::now()),
                            observed_generation: memcached.metadata.generation,
                        }],
                    };
                    let patch = serde_json::json!({ "status": status });
                    api.patch_status(memcached.metadata.name.as_deref().unwrap(), &PatchParams::default(), &Patch::Merge(&patch))
                        .await
                        .map_err(Error::KubeError)?;
            
                    return Err(Error::KubeError(e));
                }
            }
        }
        
        Some(mut found) => {
            if found.spec.replicas == None || found.spec.replicas.unwrap() != memcached.spec.size  {
                found.spec.replicas = Some(memcached.spec.size);

                match deployments.replace(memcached.metadata.name.as_deref().unwrap(), &PostParams::default(), &found).await {
                    Ok(_) => {
                        // Now, that we update the size we want to requeue the reconciliation
                		// so that we can ensure that we have the latest state of the resource before
                		// update. Also, it will help ensure the desired state on the cluster
                        return Ok(Action::requeue(Duration::from_secs(60)));
                    }
                    Err(e) => {
                        println!("Failed to udpate deployment");
                        let status = MemcachedStatus {
                            conditions: vec![Condition {
                                type_: "Available".into(),
                                status: "False".into(),
                                reason: "Resizing".into(),
                                message: "Failed to update the size for the custom resource".into(),
                                last_transition_time: Time(Timestamp::now()),
                                observed_generation: memcached.metadata.generation,
                            }],
                        };
                        let patch = serde_json::json!({ "status": status });
                        api.patch_status(memcached.metadata.name.as_deref().unwrap(), &PatchParams::default(), &Patch::Merge(&patch))
                            .await
                            .map_err(Error::KubeError)?;                        
                        return Err(Error::KubeError(e));
                    }
                }        
            }
        }
    }

    let status = MemcachedStatus {
        conditions: vec![Condition {
            type_: "Available".into(),
            status: "True".into(),
            reason: "Reconciling".into(),
            message: "Deployment for custom resource success".into(),
            last_transition_time: Time(Timestamp::now()),
            observed_generation: memcached.metadata.generation,
        }],
    };
    let patch = serde_json::json!({ "status": status });
    api.patch_status(memcached.metadata.name.as_deref().unwrap(), &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .map_err(Error::KubeError)?;        
    
    Ok(Action::await_change())
}

/// Équivalent de `deploymentForMemcached`
fn deployment_for_memcached(memcached: &Memcached) -> Deployment {
    let image = "memcached:1.6.26-alpine3.19";
    let labels = BTreeMap::from([("app.kubernetes.io/name".to_string(), "project".to_string())]);
    let oref = memcached.controller_owner_ref(&()).unwrap();

    Deployment {
        metadata: ObjectMeta {
            name: memcached.metadata.name.clone(),
            namespace: memcached.metadata.namespace.clone(),
            owner_references: Some(vec![oref]),
            ..ObjectMeta::default()
        },
        spec: DeploymentSpec {
            replicas: Some(memcached.spec.size),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..LabelSelector::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..ObjectMeta::default()
                }),
                spec: Some(PodSpec {
                    security_context: Some(PodSecurityContext {
                        run_as_non_root: Some(true),
                        seccomp_profile: Some(SeccompProfile {
                            type_: "RuntimeDefault".into(),
                            ..SeccompProfile::default()
                        }),
                        ..PodSecurityContext::default()
                    }),
                    containers: vec![Container {
                        name: "memcached".into(),
                        image: Some(image.into()),
                        image_pull_policy: Some("IfNotPresent".into()),
                        security_context: Some(SecurityContext {
                            run_as_non_root: Some(true),
                            run_as_user: Some(1001),
                            allow_privilege_escalation: Some(false),
                            capabilities: Some(Capabilities {
                                drop: Some(vec!["ALL".into()]),
                                ..Capabilities::default()
                            }),
                            ..SecurityContext::default()
                        }),
                        ports: Some(vec![ContainerPort {
                            container_port: 11211,
                            name: Some("memcached".into()),
                            ..ContainerPort::default()
                        }]),
                        command: Some(
                            ["memcached", "--memory-limit=64", "-o", "modern", "-v"]
                                .map(String::from)
                                .to_vec(),
                        ),
                        ..Container::default()
                    }],
                    ..PodSpec::default()
                }),
            },
            ..DeploymentSpec::default()
        },
        ..Deployment::default()
    }
}

fn error_policy(doc: Arc<Memcached>, error: &Error, ctx: Arc<Context>) -> Action {
    println!("Reconcile failed!");
    Action::requeue(Duration::from_secs(5 * 60))
}

pub async fn run(state: State) {
    let client = Client::try_default().await.expect("failed to create kube Client");
    let memcacheds = Api::<Memcached>::all(client.clone());
    if let Err(e) = memcacheds.list(&ListParams::default().limit(1)).await {
        println!("Error !");
        std::process::exit(1);
    }
    
    Controller::new(memcacheds, watcher::Config::default().any_semantic())
        .shutdown_on_signal()
        .owns(Api::<Deployment>::all(client.clone()), watcher::Config::default())
        .run(reconcile, error_policy, state.to_context(client).await)
        .filter_map(|x| async move { std::result::Result::ok(x) })
        .for_each(|_| futures::future::ready(()))
        .await;
}