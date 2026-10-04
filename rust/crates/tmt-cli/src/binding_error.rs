//! Shared CLI error translation; binding policy remains in core.

use crate::output::Failure;
use std::error::Error;
use tmt_adapters::{host::HostError, storage::StorageError};
use tmt_core::binding::BindingError;

pub fn endpoint_failure(error: HostError) -> Failure {
    if error.socket_permission_denied() {
        return socket_failure(error);
    }
    Failure::new("RECONCILIATION_FAILED", error.to_string(), 1).caused_by(error)
}

pub fn binding_failure(error: BindingError<StorageError, HostError>) -> Failure {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, ffi::OsStr, rc::Rc, time::Instant};
    use tmt_adapters::{
        host::{CallerEnvironment, Host, OperationOptions},
        process::{CommandError, CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner},
    };

    #[derive(Clone, Default)]
    struct ExpiredLookup(Rc<Cell<usize>>);

    impl CommandRunner for ExpiredLookup {
        fn execute(&self, mut request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            self.0.set(self.0.get() + 1);
            assert_eq!(request.program, OsStr::new("tmux"));
            assert_eq!(
                request.args,
                ["display-message", "-p", "-t", "10.3", "#{pane_id}"].map(std::ffi::OsString::from)
            );
            assert!(request.input.is_empty());
            // Expire at the existing runner boundary, without racing process
            // startup or changing the production operation budget.
            request.deadline = Instant::now();
            UnixCommandRunner.execute(request)
        }
    }

    #[test]
    fn expired_target_lookup_preserves_the_cli_error_without_terminal_writes() {
        let runner = ExpiredLookup::default();
        let host = Host::for_caller_with(
            &CallerEnvironment {
                tmux: None,
                pane: None,
                process_id: u64::from(std::process::id()),
                driver_env: Default::default(),
            },
            runner.clone(),
        );
        let error = host
            .resolve_target("10.3", OperationOptions::default())
            .unwrap_err();
        assert!(!error.cleanup_failed());
        let failure = endpoint_failure(error);
        assert_eq!(failure.status, 1);
        assert_eq!(
            failure.document(),
            serde_json::json!({"error": {
                "code": "RECONCILIATION_FAILED",
                "message": "Could not execute tmux operation (ETIMEDOUT)."
            }}),
        );
        // The exact lookup is the sole command; no terminal mutation ran.
        assert_eq!(runner.0.get(), 1);
    }
}
