//! Optional provider channels: a runtime-owned way to hand a talk payload to a
//! running agent without terminal paste. The CLI only enrolls a launch and runs
//! the hidden server; the driver owns the provider's flags, wire format and
//! result classification. Delivery itself is the driver's `send`, so routing
//! and its no-fallback rules stay in `tmt_core::driver::routing`.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    io::{BufRead, Write},
    path::Path,
    time::Instant,
};
use tmt_core::endpoint::ProcessIncarnation;

/// Why a channel could not be established or used. Copy-sized so it can ride in
/// `RuntimeError` through the runtime driver port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelFault {
    /// The record or its evidence does not describe this binding's runtime.
    Mismatch,
    /// The record exists but cannot be decoded.
    InvalidRecord,
    /// The launch owner of the enrollment could not be observed, so the record
    /// is neither proven current nor proven ended.
    Unverifiable,
    /// The session opted in, but its channel has not completed the handshake
    /// (still starting, waiting at Claude's own prompts, or failed to start).
    /// Nothing may be pasted: a prompt could interpret the input as an answer.
    NotReady,
    /// The opted-in session's ready channel is absent or refused the connection.
    /// No byte moved, but the session is still opted in, so it is not pasted to.
    Unreachable,
    /// The enrollment belongs to a launch that has ended. That is not evidence
    /// that the session never opted in; only a new launch supersedes it.
    Stale,
    /// The endpoint refused the frame; nothing reached the provider.
    Refused,
    /// The payload exceeds the channel's frame bound.
    TooLarge,
    /// Anything after the connection was made: the provider may have seen it.
    Uncertain,
}

impl ChannelFault {
    /// The public error code when this fault stops a `talk` to an opted-in
    /// session. Faults without a dedicated code stay generic.
    pub fn error_code(self) -> &'static str {
        match self {
            Self::NotReady => "CHANNEL_NOT_READY",
            Self::Unreachable => "CHANNEL_UNREACHABLE",
            Self::Stale => "CHANNEL_ENROLLMENT_ENDED",
            _ => "DELIVERY_PREPARATION_FAILED",
        }
    }

    /// The reason as a sentence, for messages.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Mismatch => "The channel record does not match this binding's runtime.",
            Self::InvalidRecord => "The channel record is unreadable.",
            Self::Unverifiable => "The channel record's launch owner could not be verified.",
            Self::NotReady => "The session opted into a message channel that is not ready.",
            Self::Unreachable => "The session opted into a message channel that is not reachable.",
            Self::Stale => {
                "The session's message-channel enrollment belongs to a launch that has ended."
            }
            Self::Refused => "The channel endpoint refused the message.",
            Self::TooLarge => "The message exceeds the channel frame limit.",
            Self::Uncertain => "The channel outcome is unknown after the connection was made.",
        }
    }
}

impl fmt::Display for ChannelFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason())
    }
}

/// Failures before a channel launch: reported before any binding or spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelError {
    /// The claimed driver has no channel.
    Unsupported,
    /// The provider could not be probed within its bounds.
    ProviderUnavailable,
    /// The provider is outside the supported range.
    ProviderVersion { found: String },
    /// The endpoint path would not fit a Unix socket address.
    PathTooLong,
    /// The enrollment record could not be written.
    Enrollment,
}

impl fmt::Display for ChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("This command has no channel support."),
            Self::ProviderUnavailable => {
                formatter.write_str("The provider version could not be verified.")
            }
            Self::ProviderVersion { found } => write!(
                formatter,
                "The provider version {found:?} is outside the supported channel range."
            ),
            Self::PathTooLong => {
                formatter.write_str("The channel socket path is too long for a Unix socket.")
            }
            Self::Enrollment => {
                formatter.write_str("The channel enrollment could not be recorded.")
            }
        }
    }
}
impl std::error::Error for ChannelError {}

/// What the launch needs to enroll one session.
pub struct ChannelPlan<'a> {
    pub binding_id: &'a str,
    /// The `tmt run` process that owns this launch; the enrollment is current
    /// only while it lives and the stored binding names it as launch owner.
    pub owner: &'a ProcessIncarnation,
    /// The absolute `tmt` the provider starts as its channel server.
    pub tmt: &'a Path,
    /// Absolute directory of endpoint records; the provider's environment is
    /// never trusted to reproduce TMT's configuration lookup.
    pub directory: &'a Path,
}

/// Arguments of one channel-server process, as parsed from its argv.
pub struct ServeRequest<'a> {
    pub binding_id: &'a str,
    pub generation: &'a str,
    pub directory: &'a Path,
}

pub trait RuntimeChannel {
    /// Verify the provider before anything is bound or spawned.
    fn preflight(
        &self,
        executable: &OsStr,
        directory: &Path,
        deadline: Instant,
    ) -> Result<(), ChannelError>;

    /// Record this launch's opt-in, before the provider starts, and return the
    /// arguments appended to the user's argv. The record is what lets a later
    /// send tell "opted in, channel not ready" from "never opted in".
    fn enroll(&self, plan: &ChannelPlan<'_>) -> Result<Vec<OsString>, ChannelError>;

    /// Remove any enrollment for this binding (a launch without the channel, or
    /// the launch ending). Best effort: staleness is also detected from the
    /// dead launch owner.
    fn withdraw(&self, directory: &Path, binding_id: &str);

    /// Run the channel server until its input closes.
    fn serve(
        &self,
        request: &ServeRequest<'_>,
        input: Box<dyn BufRead + Send>,
        output: &mut dyn Write,
    ) -> std::io::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_readiness_reachability_and_ended_enrollment_have_dedicated_codes() {
        assert_eq!(ChannelFault::NotReady.error_code(), "CHANNEL_NOT_READY");
        assert_eq!(
            ChannelFault::Unreachable.error_code(),
            "CHANNEL_UNREACHABLE"
        );
        assert_eq!(ChannelFault::Stale.error_code(), "CHANNEL_ENROLLMENT_ENDED");
        for other in [
            ChannelFault::Mismatch,
            ChannelFault::InvalidRecord,
            ChannelFault::Unverifiable,
            ChannelFault::Refused,
            ChannelFault::TooLarge,
            ChannelFault::Uncertain,
        ] {
            assert_eq!(other.error_code(), "DELIVERY_PREPARATION_FAILED");
        }
        // Not ready is reported as readiness, never as a proven pending approval.
        assert!(
            !ChannelFault::NotReady
                .reason()
                .to_lowercase()
                .contains("approval")
        );
    }
}
