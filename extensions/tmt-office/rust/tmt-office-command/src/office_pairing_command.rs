//! Public pairing composition; no credentials, HTTP or OS-store dependencies.

use crate::{
    core_access::CoreAccess,
    invocation::{OfficeOperation, OutputMode},
    output::Failure,
};
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::interrupt::Interrupt;

use crate::office_companion::{
    PairingCall, PairingReply, invoke_office_pairing, invoke_office_sync,
};
use tmt_office_model::office_protocol::{OfficeError, OfficeInvocation};

pub(crate) fn sync_before_operation(executable: &Path) -> Result<(), Failure> {
    match invoke_office_sync(executable, Instant::now() + Duration::from_secs(30)) {
        Ok(Ok(report)) if report.pending == 0 => Ok(()),
        _ => writeln!(io::stderr().lock(), "Office retirement cleanup remains unconfirmed. Run tmt office sync to retry; the requested operation is checked separately.").map_err(unavailable),
    }
}

pub fn run(
    executable: &Path,
    operation: OfficeOperation,
    mode: OutputMode,
    core_access: &dyn CoreAccess,
) -> Result<u8, Failure> {
    if matches!(operation, OfficeOperation::Sync) {
        return sync(executable, mode);
    }
    let interrupt = Interrupt::install().map_err(unavailable)?;
    // The Office boundary alone consumes its hooks. Ordinary identity commands
    // commit notifications without loading a companion or waiting for a network.
    if matches!(
        operation,
        OfficeOperation::Pair { .. } | OfficeOperation::Inspect { .. }
    ) {
        sync_before_operation(executable)?;
        if interrupt.is_interrupted() {
            return Err(interrupted());
        }
    }
    let inspect = matches!(&operation, OfficeOperation::Inspect { .. });
    let unpair = matches!(&operation, OfficeOperation::Unpair { .. });
    let (world, identity, emulator, read_only, timeout, pair) = match operation {
        OfficeOperation::Pair {
            world,
            identity,
            emulator,
            read_only,
            timeout_seconds,
        } => (world, identity, emulator, read_only, timeout_seconds, true),
        OfficeOperation::PairStatus {
            world,
            identity,
            emulator,
        } => (world, identity, emulator, false, 30, false),
        OfficeOperation::Inspect {
            world,
            identity,
            emulator,
        } => (world, identity, emulator, false, 30, false),
        OfficeOperation::Unpair {
            world,
            identity,
            emulator,
        } => (world, identity, emulator, false, 30, false),
        _ => {
            return Err(Failure::new(
                "USAGE_ERROR",
                "Expected an Office pairing operation.",
                1,
            ));
        }
    };
    let identity = core_access.identity(identity.as_deref())?;
    let call = PairingCall {
        world: &world,
        identity_id: &identity.id,
        emulator,
        read_only,
    };
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut operation = if pair {
        OfficeInvocation::PairBegin
    } else if unpair {
        OfficeInvocation::Unpair
    } else if inspect {
        OfficeInvocation::Inspect
    } else {
        OfficeInvocation::PairStatus
    };
    loop {
        if interrupt.is_interrupted() {
            return Err(interrupted());
        }
        if Instant::now() >= deadline {
            return Err(pairing_error(OfficeError::PairingPending));
        }
        let result = invoke_office_pairing(
            executable,
            operation,
            &call,
            deadline.min(Instant::now() + Duration::from_secs(30)),
        );
        if interrupt.is_interrupted() {
            return Err(interrupted());
        }
        let reply = result.map_err(|_| Failure::new("OFFICE_REMOTE_UNCERTAIN", "Office did not confirm the operation. Retry the same pairing; no new approval was requested.", 1))?;
        match reply {
            PairingReply::Pending(link) if unpair => {
                writeln!(io::stderr().lock(), "{link}").map_err(unavailable)?;
                return Err(Failure::new(
                    "OFFICE_OWNER_CANCELLATION_REQUIRED",
                    "Cancellation is unconfirmed. Ask the owner to cancel this request using the link, then retry unpair. Retained credentials were not removed.",
                    1,
                ));
            }
            PairingReply::State(state) if unpair && state == "revoked" => {
                let value =
                    serde_json::json!({"state":"revoked","identityId":identity.id,"world":world});
                writeln!(
                    io::stdout().lock(),
                    "{}",
                    if mode.json {
                        value.to_string()
                    } else {
                        "Office pairing revoked. Workspace content is retained.".into()
                    }
                )
                .map_err(unavailable)?;
                return Ok(0);
            }
            PairingReply::Inspected(exists) if inspect => {
                let value = serde_json::json!({"blockExists":exists,"identityId":identity.id,"world":world,"serverAuthorizationChecked":true});
                writeln!(
                    io::stdout().lock(),
                    "{}",
                    if mode.json {
                        value.to_string()
                    } else {
                        format!("Office access confirmed. Assigned block exists: {exists}.")
                    }
                )
                .map_err(unavailable)?;
                return Ok(0);
            }
            PairingReply::Inspected(_) => {
                return Err(pairing_error(OfficeError::CredentialsInvalid));
            }
            PairingReply::Pending(link) if pair => {
                if mode.json {
                    writeln!(io::stderr().lock(), "{link}")
                        .and_then(|()| io::stderr().flush())
                        .map_err(unavailable)?;
                } else {
                    writeln!(
                        io::stdout().lock(),
                        "Approve this identity in your browser:\n{link}"
                    )
                    .and_then(|()| io::stdout().flush())
                    .map_err(unavailable)?;
                }
            }
            PairingReply::State(state)
                if !inspect && !unpair && (!pair || state == "credential") =>
            {
                let value = serde_json::json!({"state":state,"identityId":identity.id,"world":world,"serverAuthorizationChecked":false});
                writeln!(io::stdout().lock(), "{}", if mode.json { value.to_string() } else { format!("Office pairing: {state}. Server authorization is checked when accessing a resource.") }).map_err(unavailable)?;
                return Ok(0);
            }
            PairingReply::Error(OfficeError::PairingPending | OfficeError::RemoteUncertain)
                if pair && operation == OfficeInvocation::PairPoll => {}
            PairingReply::Error(error) => return Err(pairing_error(error)),
            PairingReply::Pending(_) | PairingReply::State(_) => {
                return Err(pairing_error(OfficeError::CredentialsInvalid));
            }
        }
        operation = OfficeInvocation::PairPoll;
        interrupt
            .wait_until(deadline.min(Instant::now() + Duration::from_secs(5)))
            .map_err(unavailable)?;
    }
}

fn sync(executable: &Path, mode: OutputMode) -> Result<u8, Failure> {
    let interrupt = Interrupt::install().map_err(unavailable)?;
    let result = invoke_office_sync(executable, Instant::now() + Duration::from_secs(30));
    if interrupt.is_interrupted() {
        return Err(interrupted());
    }
    let report = result.map_err(unavailable)?.map_err(pairing_error)?;
    let value = serde_json::json!({"completed":report.completed,"failed":report.failed,"pending":report.pending,"failureCode":report.failure.map(|error| error.code())});
    writeln!(
        io::stdout().lock(),
        "{}",
        if mode.json {
            value.to_string()
        } else {
            format!(
                "Office retirement hooks: {} completed, {} failed, {} pending.{}",
                report.completed,
                report.failed,
                report.pending,
                if report.pending > 0 {
                    " Run tmt office sync to retry; remote cleanup is not yet complete."
                } else {
                    ""
                }
            )
        }
    )
    .map_err(unavailable)?;
    Ok(u8::from(report.pending > 0 || report.failed > 0))
}

pub(crate) fn pairing_error(error: OfficeError) -> Failure {
    let message = match error {
        OfficeError::PairingPending => {
            "Pairing is not yet confirmed. Retry the same command within the original approval window."
        }
        OfficeError::CredentialsUnavailable => {
            "The OS credential store or local Office state is unavailable. Unlock or configure your OS credential store before pairing."
        }
        OfficeError::PairingExpired => {
            "The retained pairing or grant has expired. No new pairing was created."
        }
        OfficeError::NotPaired => "This active identity has no Office pairing.",
        OfficeError::Busy => {
            "Another Office operation holds the local lock. This operation did not start; retry the same command after it finishes."
        }
        OfficeError::LayoutInvalid => {
            "The Office block layout is invalid or exceeds the supported bounds. No change was made."
        }
        OfficeError::ProfileInvalid => {
            "The Office presentation profile is invalid or exceeds the supported bounds. No change was made."
        }
        OfficeError::RevisionConflict => {
            "The Office block changed after it was read. Read the current layout and reconcile your draft before submitting a new revision. No layout was changed by this attempt."
        }
        _ => {
            "Office could not validate the operation. Retained pairing state was not intentionally deleted."
        }
    };
    Failure::new(error.code(), message, 1)
}

fn unavailable(error: impl std::error::Error + 'static) -> Failure {
    Failure::new(
        "OFFICE_IO_ERROR",
        "Could not access local Office state or output.",
        1,
    )
    .caused_by(error)
}

pub(crate) fn interrupted() -> Failure {
    Failure::new(
        "OFFICE_INTERRUPTED",
        "Office pairing interrupted. Retry the same command to inspect or resume retained state.",
        130,
    )
}
