//! The TMT driver protocol, version 1: how `tmt` asks an out-of-process
//! driver about the terminal host it knows. `contracts/driver-protocol-v1.md`
//! owns the wire format; this crate encodes it for both sides:
//!
//! - core decodes a driver's answer with [`decode`], which bounds, parses and
//!   validates it against the grammar the driver declared, so nothing a
//!   driver prints reaches core unchecked;
//! - a driver answers one invocation with [`serve`];
//! - [`conformance`] checks a driver against the contract through any way of
//!   invoking it.
//!
//! The crate depends on no TMT crate: a driver needs only this and
//! `serde_json`.

mod decode;
mod grammar;
mod serve;
mod wire;

pub mod conformance;

pub use decode::{Answer, DecodeError, decode, decode_capabilities, decode_done};
pub use grammar::{Grammar, GrammarError};
pub use serve::{Handler, serve};
pub use wire::*;

use std::time::Duration;

/// The protocol this crate speaks.
pub const PROTOCOL: u32 = 1;

/// The hidden subcommand every driver executable answers.
pub const SUBCOMMAND: &str = "__tmt-driver";

/// Set on every driver call; a `tmt` that sees it refuses to write, so a
/// driver that runs `tmt` cannot recurse into itself or change state.
pub const CALL_ENV: &str = "TMT_DRIVER_CALL";

/// The most a request may be; `input` text is the largest part.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// One host-driver operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Op {
    Capabilities,
    Caller,
    Server,
    ResolveTarget,
    Snapshot,
    Probe,
    Publish,
    Clear,
    Capture,
    Input,
    Focus,
}

/// How long an operation may take and how much it may print. Anything over
/// either is a failure, never a partial answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpLimits {
    pub deadline: Duration,
    pub max_output_bytes: usize,
}

const SMALL: usize = 4 * 1024;
const LARGE: usize = 1024 * 1024;

impl Op {
    pub const ALL: [Self; 11] = [
        Self::Capabilities,
        Self::Caller,
        Self::Server,
        Self::ResolveTarget,
        Self::Snapshot,
        Self::Probe,
        Self::Publish,
        Self::Clear,
        Self::Capture,
        Self::Input,
        Self::Focus,
    ];

    /// The name on the command line and in `capabilities.ops`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Capabilities => "capabilities",
            Self::Caller => "caller",
            Self::Server => "server",
            Self::ResolveTarget => "resolve-target",
            Self::Snapshot => "snapshot",
            Self::Probe => "probe",
            Self::Publish => "publish",
            Self::Clear => "clear",
            Self::Capture => "capture",
            Self::Input => "input",
            Self::Focus => "focus",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.as_str() == name)
    }

    /// Operations on a provider hook's path get 300 ms; those that read
    /// every pane or paste text get 2 s.
    pub const fn bounds(self) -> OpLimits {
        let (millis, max_output_bytes) = match self {
            Self::Capabilities => (1000, SMALL),
            Self::Caller
            | Self::Server
            | Self::ResolveTarget
            | Self::Publish
            | Self::Clear
            | Self::Focus => (300, SMALL),
            Self::Snapshot | Self::Probe | Self::Capture => (2000, LARGE),
            Self::Input => (2000, SMALL),
        };
        OpLimits {
            deadline: Duration::from_millis(millis),
            max_output_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_names_round_trip_and_are_unique() {
        for op in Op::ALL {
            assert_eq!(Op::parse(op.as_str()), Some(op));
        }
        let names: std::collections::BTreeSet<_> = Op::ALL.iter().map(|op| op.as_str()).collect();
        assert_eq!(names.len(), Op::ALL.len());
        assert_eq!(Op::parse("status"), None, "status is core's, not an op");
    }

    #[test]
    fn hook_path_ops_are_short_and_small() {
        for op in [Op::Caller, Op::Server, Op::Publish, Op::Clear] {
            assert_eq!(op.bounds().deadline, Duration::from_millis(300));
            assert_eq!(op.bounds().max_output_bytes, 4096);
        }
        assert_eq!(Op::Snapshot.bounds().max_output_bytes, 1024 * 1024);
    }
}
