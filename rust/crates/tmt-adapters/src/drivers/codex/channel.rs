//! Launcher port composition; registration awaits integrated acceptance.
use super::{attachment::LaunchOptions, delivery, supervisor::Supervisor};
use crate::{
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
    runtime::channel::{
        ChannelEnrollment, ChannelError, ChannelFault, ChannelPlan, EnrollmentReport,
        EvidenceError, Recovery, RecoveryError, RuntimeChannel,
    },
};
use std::{path::Path, time::Instant};

pub struct CodexChannel;
impl RuntimeChannel for CodexChannel {
    fn enabled_by_default(&self) -> bool {
        false
    }
    fn enrolled(&self, directory: &Path, binding_id: &str) -> Result<bool, ChannelFault> {
        delivery::enrolled(directory, binding_id)
    }
    fn enrolled_in_pane(
        &self,
        directory: &Path,
        pane: &crate::runtime::channel::PaneAddress<'_>,
        binding_id: Option<&str>,
        deadline: Instant,
    ) -> Result<crate::runtime::channel::PaneEvidence, crate::runtime::channel::EvidenceError> {
        super::pane::enrolled(directory, pane, binding_id, deadline)
    }
    fn inspect(
        &self,
        directory: &Path,
        binding_id: &str,
        deadline: Instant,
    ) -> Result<Option<EnrollmentReport>, EvidenceError> {
        super::recovery::inspect(&UnixCommandRunner, directory, binding_id, deadline)
    }
    fn recover(
        &self,
        directory: &Path,
        binding_id: &str,
        generation: &str,
        deadline: Instant,
    ) -> Result<Recovery, RecoveryError> {
        super::recovery::recover(
            &UnixCommandRunner,
            directory,
            binding_id,
            generation,
            deadline,
        )
    }
    fn preflight(
        &self,
        command: &crate::runtime::RuntimeCommand,
        working_directory: Option<&Path>,
        directory: &Path,
        deadline: Instant,
    ) -> Result<Option<String>, ChannelError> {
        check_provider(
            &UnixCommandRunner,
            command,
            working_directory,
            directory,
            deadline,
            crate::skill_installation::ProviderEnvironment::capture()
                .ok()
                .as_ref(),
        )
    }

    fn serve(
        &self,
        request: &crate::runtime::channel::ServeRequest<'_>,
        input: Box<dyn std::io::BufRead + Send>,
        output: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        super::supervisor::serve(request, input, output)
    }

    fn enroll(&self, plan: &ChannelPlan<'_>) -> Result<Box<dyn ChannelEnrollment>, ChannelError> {
        super::record::Attribution::new(
            plan.identity_id,
            plan.pane.server,
            plan.pane.pane_id,
            plan.pane.pane_pid,
        )
        .map_err(|_| ChannelError::Unattributed)?;
        LaunchOptions::for_launch(plan.command, plan.working_directory, plan.resume_session).map_err(|error| {
            use super::attachment::AttachmentError;
            ChannelError::UnsupportedArguments(match error {
                AttachmentError::UnsupportedPermission(option) => option,
                AttachmentError::UnsupportedConfig => "--config/-c key is not supported for channel attachment; use --sandbox or --ask-for-approval for permissions",
                _ => "channel launch requires supported options and no prompt/resume/fork",
            })
        })?;
        Supervisor::start(plan)
            .map(|lease| Box::new(lease) as Box<dyn ChannelEnrollment>)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    ChannelError::Occupied
                } else {
                    ChannelError::Enrollment
                }
            })
    }
}

fn check_provider(
    runner: &impl CommandRunner,
    command: &crate::runtime::RuntimeCommand,
    working_directory: Option<&Path>,
    directory: &Path,
    deadline: Instant,
    environment: Option<&crate::skill_installation::ProviderEnvironment>,
) -> Result<Option<String>, ChannelError> {
    if !directory.is_absolute() {
        return Err(ChannelError::Enrollment);
    }
    let output = runner
        .execute(CommandRequest {
            program: &command.executable,
            args: &["--version".into()],
            input: &[],
            deadline,
            max_output_bytes: 4096,
        })
        .map_err(|_| ChannelError::ProviderUnavailable)?;
    version_advisory(&output.stdout)?;
    Ok(environment
        .zip(working_directory)
        .and_then(|(environment, working_directory)| {
            trust_advisory(command, working_directory, environment)
        })
        .map(str::to_owned))
}

fn trust_advisory(
    command: &crate::runtime::RuntimeCommand,
    working_directory: &Path,
    environment: &crate::skill_installation::ProviderEnvironment,
) -> Option<&'static str> {
    let options = LaunchOptions::parse(command, working_directory).ok()?;
    match super::trust::local_project_trust(environment, options.working_directory()) {
        super::trust::LocalProjectTrust::Unset | super::trust::LocalProjectTrust::Untrusted => {
            Some("Codex may ask you to trust this folder in its own window.")
        }
        super::trust::LocalProjectTrust::Trusted | super::trust::LocalProjectTrust::Unknown => None,
    }
}

fn version_advisory(output: &[u8]) -> Result<Option<String>, ChannelError> {
    let found = String::from_utf8_lossy(output).trim().to_owned();
    let parts = found.strip_prefix("codex-cli ").and_then(|version| {
        version
            .split('.')
            .map(|part| {
                (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                    .then(|| part.parse::<u64>().ok())
                    .flatten()
            })
            .collect::<Option<Vec<_>>>()
    });
    match parts.as_deref() {
        Some([0, 159, 2 | 3] | [0, 160, 0]) => Ok(None),
        Some([0, 159, patch]) if *patch > 3 => Err(ChannelError::ProviderUnqualified {
            reason: format!(
                "Codex build {found:?} has not been qualified for the message channel."
            ),
        }),
        _ => Err(ChannelError::ProviderVersion { found }),
    }
}

#[cfg(test)]
mod tests;
