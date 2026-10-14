//! Serve owns the project runtime and its independent persistence lifetime.
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
use tmt_remote::{
    deploy_discovery::{DeclarationSource, sealed_declaration},
    error::RemoteError,
    firebase_auth::{
        AuthOwner, Clock, FileStore, Http, Jitter, ProductionHttp, Project, RandomJitter,
        SystemClock,
    },
    firestore_budget::{Decision, Model, NATIVE_DEFAULT_MODEL},
    limits,
    mount::{Extension, ObjectDeclaration},
    native_sync::{NativeBudgetFile, NativeClient, NativeRuntime},
    object_service::{
        ObjectService,
        sync::{BindInput, ClientFactory, Registration},
    },
    remote_selector,
    sealed_sync::{AuthPort, SealedSyncPort, SyncContext, SyncError},
    state::Layout,
};

pub(super) struct NativePorts {
    pub(super) http: Arc<dyn Http + Send + Sync>,
    pub(super) clock: Arc<dyn Clock + Send + Sync>,
    pub(super) jitter: Arc<dyn Jitter + Send + Sync>,
}
pub(super) struct NativeInputs<'source> {
    pub(super) source: &'source mut dyn DeclarationSource,
    pub(super) ports: Option<NativePorts>,
}
impl NativePorts {
    pub(super) fn production(project: &Project) -> Result<Self, RemoteError> {
        let clock = Arc::new(SystemClock::default());
        let http = ProductionHttp::new(project, clock.clone())
            .map_err(|error| RemoteError::new(error.code(), &error.to_string()))?;
        if http.emulator_active() {
            super::object_warning(
                "tmt-remote: test-emulator routing active (loopback, demo project)",
            );
        }
        Ok(Self {
            http: Arc::new(http),
            clock,
            jitter: Arc::new(RandomJitter),
        })
    }
}
fn unavailable() -> RemoteError {
    RemoteError::new(
        "REMOTE_STATE_UNAVAILABLE",
        "Native sealed sync is unavailable.",
    )
}
fn client_factory(
    runtime: Arc<NativeRuntime>,
    model: Model,
    auth: Arc<AuthOwner>,
) -> ClientFactory {
    Arc::new(move |input: BindInput<'_>| {
        if input.auth.identity() != auth.identity() {
            return Err(SyncError::Denied);
        }
        let context = SyncContext::bind(
            input.public_config,
            input.declaration,
            input.declaration_digest,
            input.space,
            input.auth,
        )?;
        let client = Arc::new(NativeClient::new(context, runtime.clone(), model));
        if let Decision::Refuse(refusal) = client.budget_state()? {
            return Err(SyncError::BudgetExhausted(refusal));
        }
        Ok(client as Arc<dyn SealedSyncPort>)
    })
}

pub(super) struct ProjectOwner<'objects, 'lease> {
    objects: &'objects ObjectService<'lease>,
    auth: Arc<AuthOwner>,
    runtime: Arc<NativeRuntime>,
    tick: Option<CheckpointOwner>,
    pub(super) permission_notice: Option<&'static str>,
}
impl<'objects, 'lease> ProjectOwner<'objects, 'lease> {
    pub(super) fn prepare(
        objects: &'objects ObjectService<'lease>,
        layout: &Layout,
        extensions: &[Extension],
        source: &mut dyn DeclarationSource,
        ports: Option<NativePorts>,
    ) -> Result<Option<Self>, RemoteError> {
        let Some(selected) = remote_selector::client_status(layout, None, None)? else {
            return Ok(None);
        };
        let mut declarations = Vec::new();
        for extension in extensions
            .iter()
            .filter(|extension| extension.objects == ObjectDeclaration::Local)
        {
            if let Some(reply) = source
                .declaration(extension.name)
                .map_err(|_| unavailable())?
            {
                let (bytes, digest) =
                    sealed_declaration(extension.name, &reply).map_err(|_| unavailable())?;
                declarations.push((extension.name, bytes, digest));
            }
        }
        if declarations.is_empty() {
            return Ok(None);
        }
        let model = Model::new(NATIVE_DEFAULT_MODEL.0, NATIVE_DEFAULT_MODEL.1)
            .map_err(|_| unavailable())?;
        let project =
            Project::from_public_config(&selected.public_config).map_err(|_| unavailable())?;
        let ports = match ports {
            Some(ports) => ports,
            None => NativePorts::production(&project)?,
        };
        let auth = Arc::new(
            AuthOwner::load(
                project,
                Arc::new(FileStore::new(layout, &selected.project).map_err(|_| unavailable())?),
                ports.http.clone(),
                ports.clock.clone(),
                ports.jitter.clone(),
            )
            .map_err(|_| unavailable())?,
        );
        let permission_notice = auth.take_permission_notice();
        let now = ports.clock.now();
        let budget = Arc::new(Mutex::new(
            NativeBudgetFile::open(layout, &selected.project, now.wall_ms, now.monotonic_ms)
                .map_err(|_| unavailable())?,
        ));
        let runtime = Arc::new(NativeRuntime::new(
            ports.http,
            ports.clock,
            ports.jitter,
            budget,
        ));
        let factory = client_factory(runtime.clone(), model, auth.clone());
        let mut registrations = Vec::new();
        for (name, bytes, digest) in declarations {
            registrations.push((
                name,
                Registration::new(
                    selected.public_config.clone(),
                    bytes,
                    digest,
                    auth.clone(),
                    factory.clone(),
                )
                .map_err(|_| unavailable())?,
            ));
        }
        let failed_runtime = runtime.clone();
        let failed_auth = auth.clone();
        let tick = CheckpointOwner::start(
            runtime.clone(),
            Duration::from_millis(limits::NATIVE_BUDGET_FLUSH_MS),
            Arc::new(move || {
                failed_runtime.stop();
                failed_auth.stop();
            }),
        )
        .map_err(|_| unavailable())?;
        for (name, registration) in registrations {
            if objects.register_native(name, registration).is_err() {
                runtime.stop();
                auth.stop();
                let _ = tick.finish();
                return Err(unavailable());
            }
        }
        Ok(Some(Self {
            objects,
            auth,
            runtime,
            tick: Some(tick),
            permission_notice,
        }))
    }
    pub(super) fn cancel(&self) {
        self.runtime.stop();
        self.auth.stop();
        if let Some(tick) = &self.tick {
            tick.cancel();
        }
    }
    pub(super) fn finish(mut self) -> Result<(), RemoteError> {
        self.cancel();
        self.objects.shutdown();
        self.tick
            .take()
            .expect("project tick")
            .finish()
            .map_err(|_| unavailable())
    }
}
impl Drop for ProjectOwner<'_, '_> {
    fn drop(&mut self) {
        self.cancel();
        if let Some(tick) = self.tick.take() {
            // Early startup failure closes and joins channels before persistence too.
            self.objects.shutdown();
            let _ = tick.finish();
        }
    }
}

pub(super) struct CheckpointOwner {
    wake: Arc<(Mutex<bool>, Condvar)>,
    worker: Option<JoinHandle<Result<(), SyncError>>>,
    failure: Option<SyncError>,
    runtime: Option<Arc<NativeRuntime>>,
}
impl CheckpointOwner {
    pub(super) fn start(
        runtime: Arc<NativeRuntime>,
        interval: Duration,
        failed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, SyncError> {
        if interval.is_zero() {
            return Err(SyncError::Invalid);
        }
        let wake = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_wake = wake.clone();
        let worker_runtime = runtime.clone();
        let worker = thread::Builder::new()
            .name("remote-native-budget".into())
            .spawn(move || {
                let (state, changed) = &*worker_wake;
                loop {
                    let result = (|| {
                        let stopped = state.lock().map_err(|_| SyncError::Unavailable)?;
                        let (stopped, _) = changed
                            .wait_timeout_while(stopped, interval, |stopped| !*stopped)
                            .map_err(|_| SyncError::Unavailable)?;
                        if *stopped {
                            return Ok(false);
                        }
                        drop(stopped);
                        worker_runtime.checkpoint()?;
                        Ok(true)
                    })();
                    match result {
                        Ok(true) => {}
                        Ok(false) => return Ok(()),
                        Err(error) => {
                            failed();
                            return Err(error);
                        }
                    }
                }
            })
            .map_err(|_| SyncError::Unavailable)?;
        Ok(Self {
            wake,
            worker: Some(worker),
            failure: None,
            runtime: Some(runtime),
        })
    }
    pub(super) fn cancel(&self) {
        let (state, changed) = &*self.wake;
        *state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        changed.notify_all();
    }
    pub(super) fn join(&mut self) -> Result<(), SyncError> {
        if let Some(worker) = self.worker.take() {
            self.failure = worker.join().unwrap_or(Err(SyncError::Unavailable)).err();
        }
        self.failure.map_or(Ok(()), Err)
    }
    pub(super) fn finish(mut self) -> Result<(), SyncError> {
        self.cancel();
        let joined = self.join();
        // Even a failed checkpoint must make the final persistence attempt;
        // neither failure can silently release a fresh budget allowance.
        let flushed = self.runtime.take().expect("owner runtime").finish();
        joined.and(flushed)
    }
}
impl Drop for CheckpointOwner {
    fn drop(&mut self) {
        self.cancel();
        let _ = self.join();
        if let Some(runtime) = self.runtime.take() {
            let _ = runtime.finish();
        }
    }
}
