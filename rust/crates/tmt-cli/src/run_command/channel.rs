//! The launcher's side of a `--channel` launch: verify the provider before
//! anything happens, check that this launch still holds the binding it claimed
//! before it enrolls, hold the driver's lease for the child's whole lifetime, and
//! reconcile a driver-created provider session with the admission evidence. The
//! driver owns everything provider-specific; none of it is spelled here.

use crate::{invocation::ChannelMode, output::Failure, run_command::storage_failure};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    runtime::{
        RuntimeCommand, RuntimeRegistry,
        channel::{ChannelEnrollment, ChannelError, RuntimeChannel},
    },
    storage::{Storage, StorageError},
};
use tmt_core::{
    binding::{
        Binding, BindingRepository,
        session::{HarnessId, ObservedSessionKey, ProviderSessionId, RuntimeState},
    },
    endpoint::ProcessIncarnation,
};

#[cfg(test)]
mod tests;

pub(super) fn require_enabled(requested: ChannelMode, enabled: bool) -> Result<(), Failure> {
    if requested == ChannelMode::Required && !enabled {
        return Err(Failure::new(
            "CHANNEL_DISABLED",
            "Message channels require experimental.channel=true.",
            1,
        ));
    }
    Ok(())
}

/// While experimental channels are enabled, a remembered channel is required.
/// Explicit flags override memory; admitted launches persist explicit choices.
pub(super) fn resume_mode(
    requested: ChannelMode,
    resuming: bool,
    remembered: Option<bool>,
) -> ChannelMode {
    if requested != ChannelMode::Default || !resuming {
        return requested;
    }
    match remembered {
        Some(true) => ChannelMode::Required,
        Some(false) => ChannelMode::Disabled,
        None => ChannelMode::Default,
    }
}

/// A preflight result retains the reason until the plain foreground can start,
/// so one launch prints at most one fallback notice.
pub(super) struct Prepared<'a> {
    pub channel: Option<&'a dyn RuntimeChannel>,
    pub directory: PathBuf,
    pub notice: Option<String>,
}

pub(super) fn preflight<'a>(
    registry: &'a RuntimeRegistry,
    mode: ChannelMode,
    claim: Option<&HarnessId>,
    command: &RuntimeCommand,
    paths: &ConfigPaths,
) -> Result<Prepared<'a>, Failure> {
    let working_directory = std::env::current_dir().ok();
    prepare(
        claim.and_then(|harness| registry.channel(harness)),
        mode,
        command,
        working_directory.as_deref(),
        paths.channel_directory(),
    )
}

fn prepare<'a>(
    channel: Option<&'a dyn RuntimeChannel>,
    mode: ChannelMode,
    command: &RuntimeCommand,
    working_directory: Option<&Path>,
    directory: PathBuf,
) -> Result<Prepared<'a>, Failure> {
    let mut prepared = Prepared {
        channel: None,
        directory,
        notice: None,
    };
    if mode == ChannelMode::Disabled
        || (mode == ChannelMode::Default
            && channel.is_none_or(|channel| !channel.enabled_by_default()))
    {
        return Ok(prepared);
    }
    let result: Result<&dyn RuntimeChannel, Failure> = (|| {
        let channel = channel.ok_or_else(|| {
            Failure::new(
                "CHANNEL_UNSUPPORTED",
                "This command has no message channel support.",
                1,
            )
        })?;
        let advisory = channel
            .preflight(
                command,
                working_directory,
                &prepared.directory,
                Instant::now() + Duration::from_secs(5),
            )
            .map_err(enrollment_failure)?;
        if let Some(reason) = advisory {
            super::diagnostic(&reason);
        }
        Ok(channel)
    })();
    match result {
        Ok(channel) => prepared.channel = Some(channel),
        Err(error) if mode == ChannelMode::Default => {
            prepared.notice = Some(error.message.to_string())
        }
        Err(error) => return Err(error),
    }
    Ok(prepared)
}

pub(super) fn enrollment_failure(error: ChannelError) -> Failure {
    let code = match error {
        ChannelError::Unsupported(_) => "CHANNEL_UNSUPPORTED",
        ChannelError::ProviderVersion { .. } | ChannelError::ProviderUnqualified { .. } => {
            "CHANNEL_PROVIDER_UNSUPPORTED"
        }
        _ => "CHANNEL_UNAVAILABLE",
    };
    Failure::new(code, error.to_string(), 1)
}

/// Fallback never recovers or guesses away retained enrollment. The driver's
/// failed-start cleanup must have retired its evidence before a plain launch.
pub(super) fn verify_plain_fallback(
    registry: &RuntimeRegistry,
    storage: &mut Storage,
    binding: &Binding,
    directory: &std::path::Path,
) -> Result<(), Failure> {
    verify_claim(storage, binding)?;
    let evidence = registry
        .enrolled_in_pane(
            directory,
            &tmt_adapters::runtime::channel::PaneAddress {
                server: &binding.server,
                pane_id: &binding.pane_id,
                pane_pid: binding.pane_pid,
            },
            Some(&binding.id),
            Instant::now() + Duration::from_secs(3),
        )
        .map_err(|error| Failure::new("CHANNEL_UNAVAILABLE", error.message(), 1))?;
    if evidence.enrolled {
        return Err(Failure::new(
            "CHANNEL_UNAVAILABLE",
            "A live or unconfirmed enrollment remains in this pane; no plain command was launched.",
            1,
        ));
    }
    Ok(())
}

pub(super) fn paste_notice(name: &str, reason: &str) -> String {
    // Provider output and identity names cannot turn one notice into many lines.
    let line = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{} uses paste delivery: {}", line(name), line(reason))
}

/// Whether the binding stored now is still the one this launch claimed: the same
/// session, or the documented unknown fence this launch itself wrote.
pub(super) fn claim_is_current(current: &Binding, claimed: &Binding) -> bool {
    current.session == claimed.session
        || (current.session.state == RuntimeState::Unknown
            && current.session.key == claimed.session.key
            && current.session.launch_owner == claimed.session.launch_owner)
}

/// The authority to enroll: this launch's own claim of the binding is still
/// current. Checked in a binding transaction immediately before `enroll`, so a
/// launch that lost the binding fails before any side effect.
pub(super) fn verify_claim(storage: &mut Storage, claimed: &Binding) -> Result<(), Failure> {
    let current = storage
        .with_binding_transaction::<_, StorageError>(|records| {
            Ok(records
                .entry_by_id(&claimed.identity_id)?
                .and_then(|entry| entry.binding)
                .is_some_and(|current| {
                    current.id == claimed.id && claim_is_current(&current, claimed)
                }))
        })
        .map_err(storage_failure)?;
    if current {
        Ok(())
    } else {
        Err(Failure::new(
            "RUNTIME_CONFLICT",
            "The binding changed before the message channel was enrolled; no command was launched.",
            5,
        ))
    }
}

/// The lease of one launch, held for the child's whole lifetime. It is retired
/// only by `never_spawned` (the spawn failed without a child) or `settle_wait`
/// (the same child's wait returned). Every other path out of the launch (a wait
/// error, an early return or a panic after the spawn, launcher death) drops it
/// without retiring anything, so the driver's enrollment stays exactly as it is
/// while the foreground may still run, and nothing here ever infers an end.
#[derive(Default)]
pub(super) struct HeldLease(Option<Box<dyn ChannelEnrollment>>);

impl HeldLease {
    pub(super) fn enrolled(&self) -> bool {
        self.0.is_some()
    }

    pub(super) fn hold(&mut self, lease: Box<dyn ChannelEnrollment>) {
        self.0 = Some(lease);
    }

    /// The command to spawn: the driver's plan, or the user's own without a lease.
    pub(super) fn command<'a>(&'a self, user: &'a RuntimeCommand) -> &'a RuntimeCommand {
        self.0.as_ref().map_or(user, |lease| lease.command())
    }

    /// Environment for the child only.
    pub(super) fn environment(&self) -> &[(OsString, OsString)] {
        self.0.as_ref().map_or(&[], |lease| lease.environment())
    }

    /// The provider session the driver created before the child starts.
    pub(super) fn provider_session(&self) -> Option<&ProviderSessionId> {
        self.0.as_ref().and_then(|lease| lease.provider_session())
    }

    /// The owned child exists. The driver records the exact incarnation observed
    /// for admission; without one (it could not be observed) or when the driver
    /// cannot record it, the enrollment stays unconfirmed and the reason is
    /// returned for the launcher to report. The child is kept either way.
    pub(super) fn foreground_started(
        &mut self,
        foreground: Option<&ProcessIncarnation>,
    ) -> Option<String> {
        let lease = self.0.as_mut()?;
        let Some(foreground) = foreground else {
            return Some(
                "could not observe the command's process; its channel enrollment stays unconfirmed."
                    .into(),
            );
        };
        lease.foreground_started(foreground).err().map(|error| {
            format!("could not record the command's process in its channel enrollment ({error}); the enrollment stays unconfirmed.")
        })
    }

    pub(super) fn foreground_admitted(
        &mut self,
        foreground: &ProcessIncarnation,
    ) -> Option<String> {
        self.0.as_mut()?.foreground_admitted(foreground).err().map(|error| {
            format!("command admitted, but channel readiness could not be published ({error}); its enrollment stays unready and the command keeps running.")
        })
    }

    /// The spawn failed without a child: nothing ever ran, so the enrollment ends.
    pub(super) fn never_spawned(&mut self) {
        self.retire();
    }

    /// The result of waiting for the owned child, passed through unchanged. A wait
    /// that returned reaped the same child, so the foreground is over and the
    /// enrollment ends; one that failed proves nothing, and the enrollment stays as
    /// it is. The launcher routes the wait result here before it looks at it, so this
    /// is the only place that ties the end of the enrollment to the wait.
    pub(super) fn settle_wait<T, E>(&mut self, waited: Result<T, E>) -> Result<T, E> {
        if waited.is_ok() {
            self.retire();
        }
        waited
    }

    fn retire(&mut self) {
        if let Some(lease) = self.0.take() {
            lease.withdraw();
        }
    }
}

/// The session key admitted for the foreground child. Hook evidence for this very
/// child is preserved; a driver-created session fills a key that has none and
/// never overwrites a different one (`None`: the admission fails closed). Only a
/// lease supplies the seed on a channel launch, never argv or a hook pane.
pub(super) fn admitted_key(
    existing: Option<&ObservedSessionKey>,
    child: &ProcessIncarnation,
    resumed: Option<ProviderSessionId>,
    lease: Option<&ProviderSessionId>,
) -> Option<ObservedSessionKey> {
    match existing.filter(|key| &key.incarnation == child) {
        Some(key) => match (&key.provider_session, lease) {
            (Some(recorded), Some(lease)) if recorded != lease => None,
            (None, Some(lease)) => Some(ObservedSessionKey {
                incarnation: key.incarnation.clone(),
                provider_session: Some(lease.clone()),
            }),
            _ => Some(key.clone()),
        },
        None => Some(ObservedSessionKey {
            incarnation: child.clone(),
            provider_session: resumed.or_else(|| lease.cloned()),
        }),
    }
}
