//! Optional provider channels: a runtime-owned way to hand a talk payload to a
//! running agent without terminal paste. The CLI only enrolls a launch; the
//! driver owns the provider's flags, wire format and
//! result classification. Delivery itself is the driver's `send`, so routing
//! and its no-fallback rules stay in `tmt_core::driver::routing`.

use super::RuntimeCommand;
use std::{
    ffi::{OsStr, OsString},
    fmt,
    io::{self, BufRead, Write},
    path::Path,
    time::Instant,
};
use tmt_core::{binding::session::ProviderSessionId, endpoint::ProcessIncarnation};

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
    /// The enrollment belongs to a launch that has ended. That alone is not
    /// evidence that the session never opted in: only a different launch that is
    /// positively proven current makes it non-applicable.
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
                "The session's message-channel enrollment belongs to a launch that has ended. Relaunch the agent with `tmt run` (add `--channel` to use the channel again)."
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
    /// The driver cannot plan a channel launch around this command line, and
    /// says why. The user's command is never silently rewritten or dropped.
    UnsupportedArguments(&'static str),
    /// An earlier enrollment of this binding belongs to a launch that may still
    /// be running, or whose ownership cannot be verified. It is left untouched.
    Occupied,
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
            Self::Occupied => formatter.write_str(
                "An earlier channel enrollment of this binding may still be in use or cannot be verified.",
            ),
            Self::UnsupportedArguments(reason) => {
                write!(
                    formatter,
                    "This command line cannot be launched with a channel: {reason}"
                )
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
    /// The launch as the user asked for it, resume substitution included. The
    /// driver plans the foreground command from it and never edits it in place.
    pub command: &'a RuntimeCommand,
    /// The directory the launch starts in.
    pub working_directory: &'a Path,
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

/// One launch's enrollment, held by the launcher for the whole child lifetime.
/// The provider's own state (an endpoint, a record, a generation) lives in the
/// implementation, so the launcher never inspects it and each driver decides
/// what proves that a cleanup is for this exact launch.
pub trait ChannelEnrollment {
    /// The foreground command the launcher spawns, planned by the driver from
    /// the user's command. The launcher never parses or rewrites provider
    /// arguments.
    fn command(&self) -> &RuntimeCommand;

    /// Environment for the provider child, which the subprocesses the provider
    /// itself starts may inherit. Never written to the ambient or global
    /// environment, persisted or logged.
    fn environment(&self) -> &[(OsString, OsString)];

    /// The provider session the driver created for this launch before the child
    /// starts (for example a thread its own server created), if any. The launcher
    /// records it only from here, never from argv, for the claimed harness and
    /// with the child's own incarnation, so hooks can find it. It is the mapping
    /// published at launch, not proof that the same thread stays active.
    fn provider_session(&self) -> Option<&ProviderSessionId> {
        None
    }

    /// End this launch's enrollment on every path (child exit, spawn failure).
    /// It must remove only what this launch created and never a replacement
    /// enrollment, even if the launch's own state is already gone.
    fn withdraw(self: Box<Self>);
}

pub trait RuntimeChannel {
    /// Verify the provider before anything is bound or spawned.
    fn preflight(
        &self,
        executable: &OsStr,
        directory: &Path,
        deadline: Instant,
    ) -> Result<(), ChannelError>;

    /// Record this launch's opt-in before the provider starts. The enrollment
    /// is what lets a later send tell "opted in, channel not ready" from "never
    /// opted in".
    fn enroll(&self, plan: &ChannelPlan<'_>) -> Result<Box<dyn ChannelEnrollment>, ChannelError>;

    /// Run the stdio server the provider starts as its own child, until its
    /// input closes. Only a driver whose provider starts one implements it.
    fn serve(
        &self,
        _request: &ServeRequest<'_>,
        _input: Box<dyn BufRead + Send>,
        _output: &mut dyn Write,
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "This channel has no stdio server.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

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

    use std::{cell::Cell, rc::Rc};

    struct Lease {
        command: RuntimeCommand,
        environment: Vec<(OsString, OsString)>,
        session: Option<ProviderSessionId>,
        withdrawn: Rc<Cell<u32>>,
    }

    impl ChannelEnrollment for Lease {
        fn command(&self) -> &RuntimeCommand {
            &self.command
        }

        fn environment(&self) -> &[(OsString, OsString)] {
            &self.environment
        }

        fn provider_session(&self) -> Option<&ProviderSessionId> {
            self.session.as_ref()
        }

        fn withdraw(self: Box<Self>) {
            self.withdrawn.set(self.withdrawn.get() + 1);
        }
    }

    /// A lease that overrides nothing optional.
    struct Plain;

    impl ChannelEnrollment for Plain {
        fn command(&self) -> &RuntimeCommand {
            unreachable!("not used")
        }

        fn environment(&self) -> &[(OsString, OsString)] {
            &[]
        }

        fn withdraw(self: Box<Self>) {}
    }

    struct Planner(Rc<Cell<u32>>);

    impl RuntimeChannel for Planner {
        fn preflight(&self, _: &OsStr, _: &Path, _: Instant) -> Result<(), ChannelError> {
            Ok(())
        }

        fn enroll(
            &self,
            plan: &ChannelPlan<'_>,
        ) -> Result<Box<dyn ChannelEnrollment>, ChannelError> {
            if plan
                .command
                .args
                .iter()
                .any(|argument| argument == "--prompt")
            {
                return Err(ChannelError::UnsupportedArguments("a prompt argument"));
            }
            let mut command = plan.command.clone();
            command.args.push("--planned".into());
            Ok(Box::new(Lease {
                command,
                environment: vec![("TOKEN".into(), "private".into())],
                session: Some(ProviderSessionId::new("thread-1").unwrap()),
                withdrawn: Rc::clone(&self.0),
            }))
        }
    }

    #[test]
    fn a_lease_carries_no_provider_session_unless_its_driver_created_one() {
        assert!(Plain.provider_session().is_none());
    }

    #[test]
    fn a_lease_plans_the_command_and_withdraws_exactly_once_through_the_port() {
        let withdrawn = Rc::new(Cell::new(0));
        let channel: Box<dyn RuntimeChannel> = Box::new(Planner(Rc::clone(&withdrawn)));
        let owner = ProcessIncarnation::new(1, "owner").unwrap();
        let user = RuntimeCommand {
            executable: "/bin/agent".into(),
            args: vec!["--model".into(), OsString::from_vec(vec![0xff, b'x'])],
        };
        let plan = |command| ChannelPlan {
            binding_id: "binding",
            owner: &owner,
            command,
            working_directory: Path::new("/work"),
            tmt: Path::new("/bin/tmt"),
            directory: Path::new("/channels"),
        };
        let lease = channel.enroll(&plan(&user)).unwrap();
        // The driver returns the command to spawn; the user's own is not edited.
        assert_eq!(lease.command().executable, user.executable);
        assert_eq!(lease.command().args[..2], user.args[..]);
        assert_eq!(lease.command().args[2], "--planned");
        assert_eq!(user.args.len(), 2);
        assert_eq!(
            lease.environment(),
            [(OsString::from("TOKEN"), OsString::from("private"))]
        );
        assert_eq!(
            lease.provider_session().map(ProviderSessionId::as_str),
            Some("thread-1")
        );
        assert_eq!(withdrawn.get(), 0);
        lease.withdraw();
        assert_eq!(withdrawn.get(), 1);

        let refused = RuntimeCommand {
            executable: "/bin/agent".into(),
            args: vec!["--prompt".into()],
        };
        let error = channel.enroll(&plan(&refused)).err().unwrap();
        assert_eq!(
            error,
            ChannelError::UnsupportedArguments("a prompt argument")
        );
        assert!(error.to_string().contains("a prompt argument"));
        assert_eq!(withdrawn.get(), 1);
    }
}
