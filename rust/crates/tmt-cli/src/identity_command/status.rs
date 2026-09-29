//! Status transport composition; caller resolution remains with identity commands.

use crate::{invocation::IdentityStatusRequest, output::Failure};
use serde_json::{Value, json};
use std::io::{self, Write};
use tmt_adapters::{
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use tmt_core::identity_status::{self, IdentityStatus, StatusError};

pub(super) enum Report {
    Snapshot {
        identity_id: String,
        status: Option<IdentityStatus>,
        now_ms: u64,
    },
    Cleared {
        identity_id: String,
        removed: bool,
    },
}

pub(super) fn run(
    storage: &mut Storage,
    identity_id: String,
    request: IdentityStatusRequest,
) -> Result<Report, Failure> {
    let now_ms = wall_time_ms();
    let status = match request {
        IdentityStatusRequest::Show => identity_status::show_identity_status(storage, &identity_id),
        IdentityStatusRequest::Set {
            activity,
            mood,
            ttl_ms,
        } => identity_status::set_identity_status(
            storage,
            &identity_id,
            activity,
            mood,
            now_ms,
            ttl_ms,
        )
        .map(Some),
        IdentityStatusRequest::Clear => {
            let removed =
                identity_status::clear_identity_status(storage, &identity_id).map_err(failure)?;
            return Ok(Report::Cleared {
                identity_id,
                removed,
            });
        }
    }
    .map_err(failure)?;
    Ok(Report::Snapshot {
        identity_id,
        status,
        now_ms,
    })
}

fn failure(error: StatusError<StorageError>) -> Failure {
    match error {
        StatusError::IdentityInactive => {
            Failure::new("NAME_NOT_FOUND", "The selected identity is not active.", 3)
        }
        StatusError::Invalid(error) => {
            Failure::new("IDENTITY_STATUS_INVALID", error.to_string(), 1).caused_by(error)
        }
        StatusError::Repository(error) => super::unavailable(error),
    }
}

impl Report {
    pub(super) fn value(&self) -> Value {
        match self {
            Self::Snapshot {
                identity_id,
                status,
                now_ms,
            } => json!({
                "identityId": identity_id,
                "status": tmt_adapters::identity_status::status_value(status.as_ref(), *now_ms),
            }),
            Self::Cleared {
                identity_id,
                removed,
            } => json!({ "identityId": identity_id, "removed": removed }),
        }
    }

    pub(super) fn write(
        &self,
        output: &mut impl Write,
        terminal: tmt_cli_style::Terminal,
    ) -> io::Result<()> {
        match self {
            Self::Snapshot { status: None, .. } => writeln!(output, "No self-reported status."),
            Self::Snapshot {
                status: Some(status),
                now_ms,
                ..
            } => {
                writeln!(
                    output,
                    "{}: {}",
                    if status.is_stale(*now_ms) {
                        "Stale status"
                    } else {
                        "Self-reported status"
                    },
                    status.activity
                )?;
                if let Some(mood) = &status.mood {
                    writeln!(output, "Mood: {mood}")?;
                }
                writeln!(
                    output,
                    "Updated: {} ms since epoch; expires: {} ms since epoch.",
                    status.updated_at_ms, status.expires_at_ms
                )
            }
            Self::Cleared { removed: true, .. } => {
                tmt_cli_style::message::success(output, terminal, "Cleared self-reported status")
            }
            Self::Cleared { removed: false, .. } => {
                writeln!(output, "No self-reported status to clear.")
            }
        }
    }
}
