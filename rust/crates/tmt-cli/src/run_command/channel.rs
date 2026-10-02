//! The launcher's side of a `--channel` launch: verify the provider before
//! anything happens, check that this launch still holds the binding it claimed
//! before it enrolls, hold the driver's lease for the child's whole lifetime, and
//! reconcile a driver-created provider session with the admission evidence. The
//! driver owns everything provider-specific; none of it is spelled here.

use crate::{output::Failure, run_command::storage_failure};
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
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

/// The provider channel a `--channel` launch enrolls, verified before any binding
/// or spawn. A launch that cannot enroll fails; it never silently starts without
/// the channel.
pub(super) fn preflight<'a>(
    registry: &'a RuntimeRegistry,
    claim: Option<&HarnessId>,
    executable: &OsStr,
    paths: &ConfigPaths,
) -> Result<(&'a dyn RuntimeChannel, PathBuf), Failure> {
    let unsupported = || {
        Failure::new(
            "CHANNEL_UNSUPPORTED",
            "--channel needs a command whose agent driver has a message channel (for example: tmt run --channel worker claude).",
            1,
        )
    };
    let channel = claim
        .and_then(|harness| registry.channel(harness))
        .ok_or_else(unsupported)?;
    let directory = paths.channel_directory();
    let advisory = channel
        .preflight(
            executable,
            &directory,
            Instant::now() + Duration::from_secs(5),
        )
        .map_err(|error| {
            let code = match error {
                ChannelError::ProviderVersion { .. } => "CHANNEL_PROVIDER_UNSUPPORTED",
                _ => "CHANNEL_UNAVAILABLE",
            };
            Failure::new(code, format!("{error} No command was launched."), 1)
        })?;
    // Shown before the provider takes the terminal, and only for a launch that goes on.
    if let Some(advisory) = advisory {
        super::diagnostic(&advisory);
    }
    Ok((channel, directory))
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

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "--channel needs a command whose agent driver has a message channel (for example: tmt run --channel worker claude).",
        &[")."],
        &[],
    ),
];
