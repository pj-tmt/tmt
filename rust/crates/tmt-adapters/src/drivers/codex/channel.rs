//! Launcher port composition; registration awaits integrated acceptance.
use super::{attachment::LaunchOptions, delivery, lease::Lease};
use crate::{
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
    runtime::channel::{
        ChannelEnrollment, ChannelError, ChannelFault, ChannelPlan, RuntimeChannel,
    },
};
use std::{
    ffi::OsStr,
    path::Path,
    time::{Duration, Instant},
};

pub struct CodexChannel;
impl CodexChannel {
    // The required trait method lands with Claude #715; this exact provider-local
    // implementation is consumed there without copying the unpublished port.
    pub fn enrolled(&self, directory: &Path, binding_id: &str) -> Result<bool, ChannelFault> {
        delivery::enrolled(directory, binding_id)
    }
}
impl RuntimeChannel for CodexChannel {
    fn preflight(
        &self,
        executable: &OsStr,
        directory: &Path,
        deadline: Instant,
    ) -> Result<(), ChannelError> {
        if !directory.is_absolute() {
            return Err(ChannelError::Enrollment);
        }
        let output = UnixCommandRunner
            .execute(CommandRequest {
                program: executable,
                args: &["--version".into()],
                input: &[],
                deadline,
                max_output_bytes: 4096,
            })
            .map_err(|_| ChannelError::ProviderUnavailable)?;
        let version = std::str::from_utf8(&output.stdout)
            .map_err(|_| ChannelError::ProviderUnavailable)?
            .trim();
        if !matches!(version, "codex-cli 0.159.2" | "codex-cli 0.159.3") {
            return Err(ChannelError::ProviderVersion {
                found: version.into(),
            });
        }
        // The actual owned initialize handshake qualifies again; this binary
        // preflight never qualifies an arbitrary endpoint from its self-report.
        Ok(())
    }
    fn enroll(&self, plan: &ChannelPlan<'_>) -> Result<Box<dyn ChannelEnrollment>, ChannelError> {
        LaunchOptions::parse(plan.command, plan.working_directory).map_err(|_| {
            ChannelError::UnsupportedArguments(
                "channel launch requires supported options and no prompt/resume/fork",
            )
        })?;
        Lease::start(
            plan.binding_id,
            plan.owner,
            plan.command,
            plan.working_directory,
            plan.directory,
            Instant::now() + Duration::from_secs(15),
        )
        .map(|lease| Box::new(lease) as Box<dyn ChannelEnrollment>)
        .map_err(|_| ChannelError::Enrollment)
    }
}
impl ChannelEnrollment for Lease {
    fn command(&self) -> &crate::runtime::RuntimeCommand {
        Lease::command(self)
    }
    fn environment(&self) -> &[(std::ffi::OsString, std::ffi::OsString)] {
        Lease::environment(self)
    }
    fn provider_session(&self) -> Option<&tmt_core::binding::session::ProviderSessionId> {
        self.session()
    }
    fn withdraw(mut self: Box<Self>) {
        if let Err(error) = Lease::withdraw(&mut self) {
            eprintln!("Codex lease withdrawal failed: {error}");
        }
    }
}
