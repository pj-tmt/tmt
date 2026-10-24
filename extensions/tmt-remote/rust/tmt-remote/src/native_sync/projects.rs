//! Per-serving-project Auth/runtime ownership shared by extension sync and native
//! pairing. Cold projects perform no I/O; signup is only an explicit pair action.
use super::{NativeBudgetFile, NativeRuntime};
use crate::{
    error::RemoteError,
    firebase_auth::{
        AuthOwner, AuthState, Client, Clock, FileStore, Http, Jitter, ProductionHttp, Project,
        RandomJitter, SystemClock,
    },
    state::Layout,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

#[derive(Clone)]
pub struct ProjectPorts {
    pub http: Arc<dyn Http + Send + Sync>,
    pub clock: Arc<dyn Clock + Send + Sync>,
    pub jitter: Arc<dyn Jitter + Send + Sync>,
}
impl ProjectPorts {
    pub fn production(project: &Project) -> Result<Self, RemoteError> {
        let clock = Arc::new(SystemClock::default());
        let http = ProductionHttp::new(project, clock.clone()).map_err(error)?;
        if http.emulator_active() {
            let mut output = tmt_cli_style::stream::stderr();
            let terminal = output.terminal();
            let _ = tmt_cli_style::message::warning(
                &mut output,
                terminal,
                "tmt-remote: test-emulator routing active (loopback, demo project)",
                None,
            );
        }
        Ok(Self {
            http: Arc::new(http),
            clock,
            jitter: Arc::new(RandomJitter),
        })
    }
}
pub struct ServingProject {
    pub auth: Arc<AuthOwner>,
    pub runtime: Arc<NativeRuntime>,
    config: Value,
}
pub struct Projects {
    layout: Layout,
    ports: Option<ProjectPorts>,
    entries: Mutex<HashMap<String, Arc<ServingProject>>>,
}
fn error(e: impl std::fmt::Display) -> RemoteError {
    RemoteError::new("REMOTE_STATE_UNAVAILABLE", &e.to_string())
}
impl Projects {
    pub fn new(layout: Layout, ports: Option<ProjectPorts>) -> Self {
        Self {
            layout,
            ports,
            entries: Mutex::new(HashMap::new()),
        }
    }
    pub fn load(
        &self,
        config: &Value,
        explicit_pair: bool,
    ) -> Result<Arc<ServingProject>, RemoteError> {
        let project = Project::from_public_config(config).map_err(error)?;
        let mut entries = self.entries.lock().map_err(error)?;
        if let Some(existing) = entries.get(&project.project_id).cloned() {
            if existing.config != *config {
                return Err(RemoteError::new(
                    "REMOTE_SELECTOR_INVALID",
                    "The project configuration changed.",
                ));
            }
            if let Some(auth) = existing.auth.reloaded_identity().map_err(error)? {
                existing.auth.stop();
                let replacement = Arc::new(ServingProject {
                    auth: Arc::new(auth),
                    runtime: existing.runtime.clone(),
                    config: existing.config.clone(),
                });
                entries.insert(project.project_id, replacement.clone());
                return Ok(replacement);
            }
            return Ok(existing);
        }
        let ports = match &self.ports {
            Some(ports) => ports.clone(),
            None => ProjectPorts::production(&project)?,
        };
        let store = Arc::new(FileStore::new(&self.layout, &project.project_id).map_err(error)?);
        if explicit_pair {
            let cancel = AtomicBool::new(false);
            let mut client = Client::load(
                project.clone(),
                store.as_ref(),
                ports.http.as_ref(),
                ports.clock.as_ref(),
                ports.jitter.as_ref(),
                &cancel,
            )
            .map_err(error)?;
            if client.status().state != AuthState::Active {
                client.sign_in().map_err(error)?;
            }
            client.stop();
        }
        let auth = Arc::new(
            AuthOwner::load(
                project.clone(),
                store,
                ports.http.clone(),
                ports.clock.clone(),
                ports.jitter.clone(),
            )
            .map_err(error)?,
        );
        let time = ports.clock.now();
        let budget = Arc::new(Mutex::new(
            NativeBudgetFile::open(
                &self.layout,
                &project.project_id,
                time.wall_ms,
                time.monotonic_ms,
            )
            .map_err(|_| error("Project budget unavailable"))?,
        ));
        let runtime = Arc::new(NativeRuntime::new(
            ports.http,
            ports.clock,
            ports.jitter,
            budget,
        ));
        let serving = Arc::new(ServingProject {
            auth,
            runtime,
            config: config.clone(),
        });
        entries.insert(project.project_id, serving.clone());
        Ok(serving)
    }
    pub fn checkpoint(&self) -> Result<(), RemoteError> {
        for project in self.entries.lock().map_err(error)?.values() {
            project
                .runtime
                .checkpoint()
                .map_err(|_| error("Project budget checkpoint failed"))?;
        }
        Ok(())
    }
    pub fn stop(&self) -> Result<(), RemoteError> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut failure = None;
        for project in entries.values() {
            project.runtime.stop();
            project.auth.stop();
            if project.runtime.finish().is_err() && failure.is_none() {
                failure = Some(error("Project budget flush failed"));
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for Projects {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
