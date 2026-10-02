//! Fresh command selection and exact resume settlement share the existing session policy.

use super::{Launch, Resume, diagnostic, storage_failure, warn};
use crate::output::Failure;
use std::ffi::OsString;
use tmt_adapters::{
    config::ConfigPaths,
    runtime::{RuntimeCommand, RuntimeError, RuntimeRegistry},
    storage::{Storage, StorageError},
};
use tmt_core::{
    binding::{
        BindingRepository,
        session::{RememberedSession, SessionPreferences},
    },
    driver::ActionResult,
};

fn resume_failure(code: &'static str, message: String, name: &str) -> Failure {
    Failure::new(
        code,
        format!("{message} Start fresh with: tmt run {name}"),
        1,
    )
}

/// A resume never falls back to a fresh start: an unavailable or unsupported
/// session is reported, and a fresh start stays explicit (`tmt run <name>`).
pub(super) fn select_command(
    registry: &mut RuntimeRegistry,
    name: &str,
    command: &[OsString],
    resume: Option<Resume>,
    preferences: &SessionPreferences,
) -> Result<Launch, Failure> {
    if let Some(command) = RuntimeCommand::verbatim(command) {
        return Ok(Launch {
            command,
            resumed: None,
        });
    }
    let Some(resume) = resume else {
        let command = preferences
            .preferred_harness
            .as_ref()
            .and_then(|harness| registry.relaunch(harness))
            .ok_or_else(|| {
                Failure::new(
                    "USAGE_ERROR",
                    "Specify a command, e.g. tmt run opus claude; no usable harness is remembered.",
                    1,
                )
            })?;
        return Ok(Launch {
            command,
            resumed: None,
        });
    };
    let Some(session) = &preferences.remembered else {
        return Err(resume_failure(
            "RESUME_UNAVAILABLE",
            format!("{name} has no remembered session to resume."),
            name,
        ));
    };
    let harness = session.harness.as_str();
    if session.stale_at_ms.is_some() && !resume.retry {
        return Err(resume_failure(
            "RESUME_UNAVAILABLE",
            format!(
                "{name}'s remembered {harness} session looked gone when it was last resumed. Forget it with: tmt resume --forget {name}; try it once more with: tmt resume --retry {name}."
            ),
            name,
        ));
    }
    match registry.resume(session) {
        ActionResult::Completed(command) => Ok(Launch {
            command,
            resumed: Some(session.clone()),
        }),
        ActionResult::Unsupported => Err(resume_failure(
            "RESUME_UNSUPPORTED",
            format!("The {harness} driver cannot resume {name}'s remembered session."),
            name,
        )),
        ActionResult::Failed(RuntimeError::InvalidSession) => Err(resume_failure(
            "RESUME_UNAVAILABLE",
            format!("{name}'s remembered {harness} session is not one its driver can resume."),
            name,
        )),
        ActionResult::Failed(error) => {
            Err(
                Failure::new("RUNTIME_ERROR", "Could not prepare runtime resume.", 1)
                    .caused_by(error),
            )
        }
    }
}

pub(super) fn mark_resume_pending(
    storage: &mut Storage,
    identity_id: &str,
    session: &RememberedSession,
) -> Result<(), Failure> {
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let mut preferences = records.session_preferences(identity_id)?;
            if let Some(remembered) = preferences.remembered.as_mut().filter(|remembered| {
                remembered.harness == session.harness
                    && remembered.provider_session == session.provider_session
            }) {
                remembered.resume_pending_at_ms =
                    Some(tmt_adapters::request_runtime::wall_time_ms());
                records.set_session_preferences(identity_id, &preferences)?;
            }
            Ok(())
        })
        .map_err(storage_failure)
}

/// A non-zero exit that no signal caused (128 + n), from a provider whose
/// TMT start hook was installed at launch, so its start would have been seen.
pub(super) fn failure_is_trustworthy(status: u32, hooks_installed: bool) -> bool {
    status != 0 && status <= 128 && hooks_installed
}

/// A resumed provider that exited unconfirmed marks its session stale only
/// when the failure is trustworthy: a non-zero exit that no signal caused
/// (128 + n), with the provider's TMT start hook installed at launch, so a
/// start would have been observed. The child's status stays authoritative.
pub(super) fn settle_resume(
    paths: &ConfigPaths,
    name: &str,
    identity_id: &str,
    session: &RememberedSession,
    status: u32,
    hooks_installed: bool,
) {
    let trustworthy = failure_is_trustworthy(status, hooks_installed);
    let settled = Storage::open(&paths.database).and_then(|mut storage| {
        let stale = storage.with_binding_transaction::<_, StorageError>(|records| {
            let mut preferences = records.session_preferences(identity_id)?;
            let before = preferences.clone();
            let stale = preferences.settle_resume(
                &session.provider_session,
                trustworthy,
                tmt_adapters::request_runtime::wall_time_ms(),
            );
            if preferences != before {
                records.set_session_preferences(identity_id, &preferences)?;
            }
            Ok(stale)
        });
        storage.close().and(stale)
    });
    match settled {
        Ok(true) => warn(
            &format!(
                "{} exited before confirming {name}'s resumed session; marked it stale.",
                session.harness.as_str()
            ),
            Some(&format!(
                "forget it with tmt resume --forget {name}, or try once more with tmt resume --retry {name}"
            )),
        ),
        Ok(false) => {}
        Err(_) => {
            diagnostic("could not record the resume outcome; the recorded exit is unchanged.")
        }
    }
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Specify a command, e.g. tmt run opus claude; no usable harness is remembered.",
        &[";"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "forget it with tmt resume --forget {name}, or try once more with tmt resume --retry {name}",
        &[", or", ""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{message} Start fresh with: tmt run {name}",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{name}'s remembered {harness} session looked gone when it was last resumed. Forget it with: tmt resume --forget {name}; try it once more with: tmt resume --retry {name}.",
        &[";", "."],
        &[],
    ),
];
