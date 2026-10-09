//! The background startup of an extension's serve, written once for every product.
//!
//! # Lifecycle contract
//!
//! This is the one owner of the serve lifecycle semantics; the product contracts link here.
//!
//! - **Modes.** A human invocation starts a detached serve and returns after verified readiness.
//!   `--background` does the same with `--json`, printing one typed ready or error result.
//!   `--foreground` keeps direct terminal ownership. The mode flags are mutually exclusive, and a
//!   bare `--json` stays foreground so existing supervisors keep working.
//! - **Exact worker.** The launcher starts only its own executable again, as the worker, with a
//!   private Unix socket pair as the worker's stdin. The worker starts its own session (closing
//!   the terminal does not stop it), takes the pair, closes inherited stdin and runs the same
//!   foreground composition as a foreground serve.
//! - **Readiness.** The worker sends one `Ready` record after it owns every resource a started
//!   serve owns (the product decides which) and before it serves. Serving starts only after the
//!   launcher's one-byte `Accept`, which is the launcher's no-kill cutoff. The worker answers
//!   `Accepted` once; a lost acknowledgment never revokes the handoff.
//! - **Bounded startup.** One absolute startup deadline covers every frame, including partial
//!   headers and payloads. Records are typed and byte-bounded. The deadline is not a total
//!   command-duration or filesystem-I/O promise.
//! - **Before `Accept`.** Cancellation, EOF, a deadline or an invalid record make the worker clean
//!   up and the launcher wait, within the cleanup wait, for its own worker's confirmed exit. A
//!   worker that cannot confirm cleanup is reported as startup unconfirmed: the person inspects
//!   status and uses stop, and nothing starts again automatically.
//! - **The cutoff.** The launcher's successful one-byte `Accept` write is its no-kill cutoff. A
//!   cancellation observed earlier wins; a signal racing that write may lose to the acceptance.
//!   The worker consumes a buffered `Accept` before a later EOF, preserves actual shutdown
//!   signals, attempts `Accepted` once and closes the startup endpoint.
//! - **After `Accept`.** The launcher never cleans up, kills or retries. Lost acknowledgment or
//!   lost output is reported as startup unconfirmed too, never as proof that the serve is gone.
//!   Partial readiness output is never followed by a second error record.
//! - **No kill authority from discovery.** Only the exact child the launcher spawned and has not
//!   reaped is ever signalled; no PID or status lookup supplies authority, and no signal follows a
//!   reap or an acceptance. Forced termination, a crash or an inherited lease cannot prove that
//!   every descendant ended; that uncertainty is reported, never assumed away.
//! - **Second start.** A start that finds the serve lock held reports the running serve and starts
//!   nothing; products map that to their own already-serving error.
//! - **Failure record.** After the worker holds the serve lock it may clear and then write one
//!   bounded, sanitized failure record for a failure the launcher can no longer receive. It never
//!   appends or retries, and a missing record does not prove a healthy exit.
//! - **Stop.** Stopping goes through the product's existing stop owner, never a signal to a PID.
//!   What a serve started itself (a door) stops with it; what it only attached to keeps running.
//!
//! The crate has no product names, paths, ports or policy. A product passes its program and
//! arguments, the readiness validator, the failure-record file, its `unconfirmed` and `cancelled`
//! errors and its [`Timing`].
mod frame;
mod launch;
mod record;
mod signals;
mod worker;

pub use launch::{Launch, launch};

/// The private record format, for a product's own tests that play the launcher against its
/// foreground composition. Products never speak it outside those tests.
pub mod wire {
    pub use crate::frame::{
        ACCEPT, ACCEPTED, Broken, CANCEL, FAILED, READY, read_frame, write_frame,
    };
}
pub use record::ErrorRecord;
pub use signals::Signals;
pub use worker::{Handoff, adopt};

use std::time::Duration;

/// A typed startup failure, as the product reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupError {
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
}
impl StartupError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
        }
    }
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// The bounds a product chooses for its startup.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// One absolute admission deadline for the whole handoff.
    pub startup: Duration,
    /// How long the launcher waits for its own worker's confirmed exit after a cancel.
    pub cleanup_wait: Duration,
    /// The most bytes of one private record, header included.
    pub record_bytes: usize,
}

/// What a product says when the handoff cannot settle, and the bounds it runs under.
#[derive(Debug, Clone)]
pub struct Handshake {
    /// The handoff could not be confirmed; the serve may or may not run.
    pub unconfirmed: StartupError,
    /// Startup ended before the handoff, by a signal.
    pub cancelled: StartupError,
    pub timing: Timing,
}
