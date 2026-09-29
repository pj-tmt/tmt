//! Shared CLI error translation; binding policy remains in core.

use crate::output::Failure;
use std::error::Error;
use tmt_adapters::{storage::StorageError, tmux::TmuxError};
use tmt_core::binding::BindingError;

pub fn endpoint_failure(error: TmuxError) -> Failure {
    if error.socket_permission_denied() {
        return socket_failure(error);
    }
    Failure::new("RECONCILIATION_FAILED", error.to_string(), 1).caused_by(error)
}

pub fn binding_failure(error: BindingError<StorageError, TmuxError>) -> Failure {
    // The bind error is what the user acts on; a failed cleanup is secondary.
    if let BindingError::CleanupFailed { error, cleanup } = error {
        return binding_failure(*error).with_secondary_error(cleanup);
    }
    if let BindingError::Endpoint(endpoint) = &error
        && endpoint.socket_permission_denied()
    {
        return socket_failure(error);
    }
    let (code, status) = match &error {
        BindingError::InvalidName(_) => ("INVALID_NAME", 1),
        BindingError::NameNotFound(_) => ("NAME_NOT_FOUND", 3),
        BindingError::PaneNotFound(_) => ("PANE_NOT_FOUND", 3),
        BindingError::TargetChanged(_) => ("PANE_CHANGED", 3),
        BindingError::NameAlreadyActive => ("NAME_ALREADY_ACTIVE", 5),
        BindingError::NameTaken(_) => ("NAME_ALREADY_ACTIVE", 5),
        BindingError::PaneAlreadyBound => ("PANE_ALREADY_BOUND", 5),
        BindingError::ConfirmationRequired => ("CONFIRMATION_REQUIRED", 5),
        _ => ("RECONCILIATION_FAILED", 1),
    };
    Failure::new(code, error.to_string(), status).caused_by(error)
}

pub fn socket_failure(error: impl Error + 'static) -> Failure {
    Failure::new(
        "TMUX_PERMISSION_DENIED",
        "TMT cannot access the tmux socket. An agent sandbox may be blocking it: rerun this same command with the provider's escalation, or allow the socket. No pane input occurred.",
        1,
    )
    .caused_by(error)
}
