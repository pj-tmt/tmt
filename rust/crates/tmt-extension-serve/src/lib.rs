//! The background startup of an extension's serve, written once for every product.
//!
//! # Lifecycle contract
//!
//! The normative semantics live in [`contracts/extension-serve-v1.md`](../../../../contracts/extension-serve-v1.md);
//! this crate implements them. In short: a human `serve` starts a detached worker (its own
//! executable, its own session, a private socket pair as stdin) and returns after one typed
//! `Ready` record under one startup deadline. The launcher's one-byte `Accept` is its no-kill
//! cutoff: before it, a cancelled start is cleaned up and the launcher waits for its own worker's
//! confirmed exit; after it, nothing is cleaned up, killed or retried. Only the exact unreaped
//! child is ever signalled. A held serve lock is a second start, and one bounded failure record
//! covers what the launcher can no longer receive.
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
