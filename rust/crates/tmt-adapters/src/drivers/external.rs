//! Opt-in approved runtime seam. Production registries remain built-in only.
mod lifecycle;
#[cfg(test)]
mod tests;

use crate::driver_protocol::{
    Declaration,
    process::DriverProcess,
    registry::{self, ApprovalState, DriverRecord, RegistryError},
};
use crate::process::UnixCommandRunner;
use crate::runtime::{RuntimeCommand, RuntimeError, RuntimeRegistry, driver_state};
use std::{
    fmt,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::{BindingEntry, session::HarnessId},
    driver::{ActionResult, Driver, HarnessResume},
};
use tmt_driver_protocol::{Op, ResumeRequest, ResumeResponse, RuntimeDeclaration};

type Process = DriverProcess<UnixCommandRunner, RuntimeDeclaration>;

pub struct ApprovedRuntime {
    pub(super) record: DriverRecord,
    pub(super) declaration: RuntimeDeclaration,
    process: Option<Arc<Process>>,
    pub state: ApprovalState,
    pub reason: Option<String>,
}
impl fmt::Debug for ApprovedRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApprovedRuntime")
            .field("name", &self.record.name)
            .field("state", &self.state)
            .finish()
    }
}
impl ApprovedRuntime {
    pub(super) fn load(
        global: &Path,
        record: DriverRecord,
        tmt: &Path,
    ) -> Result<Self, RegistryError> {
        let Declaration::Runtime(capabilities) = &record.capabilities else {
            return Err(RegistryError::Invalid(
                "Expected a runtime declaration.".into(),
            ));
        };
        let declaration = RuntimeDeclaration::new(capabilities)
            .map_err(|error| RegistryError::Invalid(error.to_string()))?;
        let (mut state, mut reason) = registry::state(global, &record, tmt, &UnixCommandRunner);
        let process = if state == ApprovalState::Ok {
            match Process::open_runtime(record.clone(), UnixCommandRunner) {
                Ok(process) => Some(Arc::new(process)),
                Err(error) => {
                    state = ApprovalState::Changed;
                    reason = Some(error.to_string());
                    None
                }
            }
        } else {
            None
        };
        Ok(Self {
            record,
            declaration,
            process,
            state,
            reason,
        })
    }
    pub fn available(&self) -> bool {
        self.process.is_some()
    }
    pub(crate) fn register(
        self: &Arc<Self>,
        registry: &mut RuntimeRegistry,
    ) -> Result<(), RuntimeError> {
        let harness =
            HarnessId::new(&self.record.name).map_err(|_| RuntimeError::InvalidRegistration)?;
        registry.register(
            harness.clone(),
            &self.declaration.executables()[0],
            0,
            ExternalRuntime(self.clone()),
        )?;
        registry.register_lifecycle(
            &harness,
            Box::new(lifecycle::DeclaredLifecycle(self.declaration.clone())),
        )
    }
}

struct ExternalRuntime(Arc<ApprovedRuntime>);
impl Driver for ExternalRuntime {
    type Target = BindingEntry;
    type Error = RuntimeError;
    type Launch = RuntimeCommand;
    fn claims(&self, command: &str) -> Option<HarnessId> {
        self.0
            .declaration
            .claims(command)
            .then(|| HarnessId::new(&self.0.record.name).ok())
            .flatten()
    }
    // It never sends: delivery belongs to the host and adds no observer grace.
    fn maximum_send_duration(&self) -> Duration {
        Duration::ZERO
    }
    fn resume(&mut self, resume: HarnessResume<'_>) -> ActionResult<RuntimeCommand, RuntimeError> {
        if resume.start.harness.as_str() != self.0.record.name
            || resume.start.context.is_some()
            || resume.mode.as_str() != "default"
            || !self.0.declaration.supports(Op::Resume)
        {
            return ActionResult::Unsupported;
        }
        let Some(process) = &self.0.process else {
            return ActionResult::Failed(RuntimeError::DriverUnavailable);
        };
        let request = ResumeRequest {
            session: resume.session.as_str().into(),
            model: resume.state.and_then(driver_state::state_model),
        };
        match process.call::<ResumeResponse>(request, Instant::now() + Op::Resume.bounds().deadline)
        {
            Ok(Ok(answer)) => {
                let argv = answer.argv.into_iter().map(Into::into).collect::<Vec<_>>();
                RuntimeCommand::verbatim(&argv).map_or(
                    ActionResult::Failed(RuntimeError::DriverRefused),
                    ActionResult::Completed,
                )
            }
            _ => ActionResult::Failed(RuntimeError::DriverRefused),
        }
    }
}
