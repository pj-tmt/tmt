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
//! A host driver knows a terminal host; a runtime driver knows a coding
//! agent. [`RuntimeDeclaration`] is a runtime driver's checked declaration,
//! which core applies on an agent's hook path without starting the driver.
//!
//! The crate depends on no TMT crate: a driver needs only this and
//! `serde_json`.

mod decode;
mod grammar;
mod runtime;
mod serve;
mod wire;

pub mod conformance;

pub use decode::{
    Answer, DecodeError, decode, decode_capabilities, decode_done, decode_runtime_capabilities,
};
pub use grammar::{Grammar, GrammarError, HostGrammar};
pub use runtime::{HookObservation, RuntimeDeclaration};
pub use serve::{Handler, RuntimeHandler, serve, serve_runtime};
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

/// The most a provider hook's payload may be for core to decode it.
pub const MAX_HOOK_PAYLOAD_BYTES: usize = 64 * 1024;

/// One driver operation: a host driver's, or a runtime driver's from
/// [`Op::Locations`] on. A driver answers `unsupported` to any it doesn't
/// declare, including the other kind's.
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
    Prompt,
    Focus,
    Locations,
    Resume,
    Usage,
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
    pub const ALL: [Self; 15] = [
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
        Self::Prompt,
        Self::Focus,
        Self::Locations,
        Self::Resume,
        Self::Usage,
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
            Self::Prompt => "prompt",
            Self::Focus => "focus",
            Self::Locations => "locations",
            Self::Resume => "resume",
            Self::Usage => "usage",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.as_str() == name)
    }

    /// Whether this is a runtime driver's operation.
    pub const fn is_runtime(self) -> bool {
        matches!(self, Self::Locations | Self::Resume | Self::Usage)
    }

    /// Operations on a provider hook's path get 1 s: each call is a fresh
    /// process, and a cold exec alone can take 300 ms on a busy machine.
    /// Those that read every pane or paste text get 2 s. A runtime
    /// operation gets 1 s; `usage` gets less when its hook has less left.
    pub const fn bounds(self) -> OpLimits {
        let (millis, max_output_bytes) = match self {
            Self::Capabilities => (1000, SMALL),
            Self::Caller
            | Self::Server
            | Self::ResolveTarget
            | Self::Publish
            | Self::Clear
            | Self::Focus
            | Self::Locations
            | Self::Resume
            | Self::Usage => (1000, SMALL),
            Self::Snapshot | Self::Probe | Self::Capture => (2000, LARGE),
            Self::Input | Self::Prompt => (2000, SMALL),
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
            assert_eq!(op.bounds().deadline, Duration::from_millis(1000));
            assert_eq!(op.bounds().max_output_bytes, 4096);
        }
        assert_eq!(Op::Snapshot.bounds().max_output_bytes, 1024 * 1024);
    }

    #[test]
    fn runtime_ops_are_short_small_and_marked() {
        for op in Op::ALL {
            assert_eq!(
                op.is_runtime(),
                matches!(op.as_str(), "locations" | "resume" | "usage"),
                "{op:?}"
            );
            if op.is_runtime() {
                assert_eq!(op.bounds().deadline, Duration::from_millis(1000));
                assert_eq!(op.bounds().max_output_bytes, 4096);
            }
        }
    }
}
