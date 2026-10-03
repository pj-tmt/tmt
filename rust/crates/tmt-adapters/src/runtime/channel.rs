//! Optional provider channels: a runtime-owned way to hand a talk payload to a
//! running agent without terminal paste. The CLI only enrolls a launch; the
//! driver owns the provider's flags, wire format and
//! result classification. Delivery itself is the driver's `send`, so routing
//! and its no-fallback rules stay in `tmt_core::driver::routing`.

use super::RuntimeCommand;
use std::{
    ffi::OsString,
    fmt,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use tmt_core::{
    binding::session::ProviderSessionId,
    endpoint::{ProcessIncarnation, ServerEvidence},
};

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
    /// A session in the pane has channel evidence that no active binding verifiably
    /// names, so nothing can verify whose enrollment it is and nothing may be pasted.
    Inactive,
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
            Self::Inactive => {
                "A session in this pane opted into a message channel, but no active binding verifiably names it, so nothing is pasted to it. Relaunch it with `tmt run` (add `--channel` to use the channel again)."
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

/// Pane-level evidence that cannot be told, with what a message needs to say what
/// to look at and how to recover (a fault alone is a static, copy-sized value and
/// cannot carry a path or a command).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceError {
    pub fault: ChannelFault,
    /// The record file or directory at fault.
    pub path: Option<PathBuf>,
    /// The driver's own recovery text: what to verify and the exact command.
    pub detail: Option<String>,
}

impl EvidenceError {
    pub fn at(fault: ChannelFault, path: &Path) -> Self {
        Self {
            fault,
            path: Some(path.to_owned()),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: String) -> Self {
        self.detail = Some(detail);
        self
    }

    /// The reason as a sentence, with the driver's recovery text or at least the
    /// file or directory at fault.
    pub fn message(&self) -> String {
        let mut text = self.fault.reason().to_owned();
        match (&self.detail, &self.path) {
            (Some(detail), _) => {
                text.push(' ');
                text.push_str(detail);
            }
            (None, Some(path)) => {
                text.push_str(&format!(" See {}.", path.display()));
            }
            (None, None) => {}
        }
        text
    }
}

impl From<ChannelFault> for EvidenceError {
    fn from(fault: ChannelFault) -> Self {
        Self {
            fault,
            path: None,
            detail: None,
        }
    }
}

/// The pane a launch runs in, as the launcher's own binding names it: the server
/// incarnation, the pane ID on it and the pane's process. A driver persists it in
/// its enrollment before the foreground starts, so the enrollment can be matched
/// to this pane later even when the binding is gone.
#[derive(Debug, Clone, Copy)]
pub struct PaneAddress<'a> {
    pub server: &'a ServerEvidence,
    pub pane_id: &'a str,
    pub pane_pid: u64,
}

/// What a driver knows about enrollments attributed to one pane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneEvidence {
    /// A live or unconfirmed enrollment belongs to this pane: nothing may be pasted.
    pub enrolled: bool,
    /// Records that could not be attributed to any pane (unreadable, or written
    /// without an attribution), skipped rather than blocking unrelated panes; the
    /// caller reports them by name.
    pub skipped: Vec<PathBuf>,
}

/// The part a recorded process played in an enrollment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessRole {
    /// The `tmt run` process that enrolled the launch.
    LaunchOwner,
    /// The agent the launcher spawned and published through `foreground_started`.
    Foreground,
    /// The provider process the driver recorded itself (Claude after its
    /// handshake), which is the foreground's stand-in.
    Provider,
    /// A provider endpoint the driver started and owns (the Codex app-server).
    Endpoint,
}

/// What one exact observation of a recorded incarnation (PID and start identity)
/// found. A stopped process is still present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Present,
    /// Conclusively gone: absent, or its PID now belongs to another incarnation.
    Gone,
    Unobservable,
}

/// Observes one recorded incarnation exactly, within `deadline`. Only a process
/// with the recorded PID and start identity is present; a reused PID is gone.
pub fn observe_recorded<R: crate::process::CommandRunner>(
    runner: &R,
    process: &ProcessIncarnation,
    deadline: Instant,
) -> ProcessState {
    use crate::process::runtime::{ProcessObservation, observe_runtime_process};
    match observe_runtime_process(runner, process.pid(), deadline) {
        Ok(ProcessObservation::Live(seen) | ProcessObservation::Stopped(seen)) => {
            if seen == *process {
                ProcessState::Present
            } else {
                ProcessState::Gone
            }
        }
        Ok(ProcessObservation::Gone | ProcessObservation::UnreapedZombie(_)) => ProcessState::Gone,
        Ok(ProcessObservation::Unknown) | Err(_) => ProcessState::Unobservable,
    }
}

/// Whether `value` is a binding ID or generation as enrollments store them: a
/// hyphenated UUID, which also never names a path outside the channel directory.
pub fn valid_enrollment_id(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok()
}

/// The command that shows one binding's enrollments, for diagnostics. Binding IDs
/// are UUIDs, so the words need no quoting.
pub fn inspect_command(binding_id: &str) -> String {
    format!("tmt channel inspect --binding {binding_id}")
}

/// The command that recovers exactly one enrollment, for diagnostics.
pub fn recover_command(binding_id: &str, generation: &str) -> String {
    format!("tmt channel recover --binding {binding_id} --generation {generation}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedProcess {
    pub role: ProcessRole,
    pub pid: u64,
    pub start: String,
    pub state: ProcessState,
}

/// The pane an enrollment persisted at enroll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedPane {
    pub host: String,
    pub server_id: String,
    pub socket_path: String,
    pub pane_id: String,
    pub pane_pid: u64,
}

/// What recovery may conclude about an enrollment from its own record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentState {
    /// A recorded process is present: the enrollment is in use, never recovered.
    Running,
    /// A recorded process cannot be observed: neither in use nor over.
    Unverifiable,
    /// Every recorded process is gone, but the launcher never published its
    /// foreground, so nothing TMT records proves that the agent is gone. Only the
    /// user can check the pane; recovery proceeds on their explicit request.
    Unconfirmed,
    /// The foreground was recorded and every recorded process is gone.
    Ended,
}

/// One driver's enrollment of one binding, as recovery inspects it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentReport {
    /// The driver's record file.
    pub record: PathBuf,
    pub binding_id: String,
    pub identity_id: Option<String>,
    /// The launch's generation: recovery names the exact enrollment by it.
    pub generation: String,
    pub pane: Option<RecordedPane>,
    pub processes: Vec<RecordedProcess>,
    /// Whether the record names the foreground (published by the launcher, or a
    /// recorded provider process that is itself the foreground).
    pub foreground_recorded: bool,
    /// What the user must confirm before recovering, in the driver's words.
    pub verification: String,
    /// Files recovery removes when it proceeds.
    pub removes: Vec<PathBuf>,
    /// Paths recovery leaves in place because nothing proves them unused.
    pub keeps: Vec<PathBuf>,
}

impl EnrollmentReport {
    pub fn state(&self) -> EnrollmentState {
        let any = |state| self.processes.iter().any(|process| process.state == state);
        if any(ProcessState::Present) {
            EnrollmentState::Running
        } else if any(ProcessState::Unobservable) {
            EnrollmentState::Unverifiable
        } else if self.foreground_recorded {
            EnrollmentState::Ended
        } else {
            EnrollmentState::Unconfirmed
        }
    }
}

/// The result of recovering one driver's enrollment of a binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// The binding has no enrollment of this driver: nothing to do, which also
    /// makes a repeated recovery a no-op.
    Absent,
    /// The record on file is a different generation; it is left untouched.
    OtherGeneration(EnrollmentReport),
    /// The named enrollment was removed: `removed` lists what was deleted and
    /// `kept` what was left in place.
    Recovered {
        report: EnrollmentReport,
        removed: Vec<PathBuf>,
        kept: Vec<PathBuf>,
    },
}

/// Why a named enrollment was not recovered. Nothing was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// A recorded process is present.
    Running(Box<EnrollmentReport>),
    /// A recorded process could not be observed.
    Unverifiable(Box<EnrollmentReport>),
    /// The record changed between the observation and the removal.
    Changed(Box<EnrollmentReport>),
    /// The record cannot be read, so it names nothing that could be verified;
    /// it stays manual-only, with the driver's instructions.
    Invalid(EvidenceError),
    /// The lock could not be taken or a removal failed.
    Failed { path: PathBuf },
}

/// Failures before a channel launch: reported before any binding or spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelError {
    /// The driver cannot enroll this launch and owns the reason shown to the user.
    Unsupported(&'static str),
    /// The provider could not be probed within its bounds.
    ProviderUnavailable,
    /// The provider is outside the supported range.
    ProviderVersion { found: String },
    /// The driver refuses an otherwise parseable but unqualified build.
    ProviderUnqualified { reason: String },
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
    /// The pane address cannot identify a pane later (an empty ID or a zero pid),
    /// so an enrollment could not be matched to it. Nothing is launched.
    Unattributed,
}

impl fmt::Display for ChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(reason) => formatter.write_str(reason),
            Self::ProviderUnavailable => {
                formatter.write_str("The provider version could not be verified.")
            }
            Self::ProviderVersion { found } => write!(
                formatter,
                "The provider version {found:?} is outside the supported channel range."
            ),
            Self::ProviderUnqualified { reason } => formatter.write_str(reason),
            Self::PathTooLong => {
                formatter.write_str("The channel socket path is too long for a Unix socket.")
            }
            Self::Enrollment => {
                formatter.write_str("The channel enrollment could not be recorded.")
            }
            Self::Occupied => formatter.write_str(
                "An earlier channel enrollment of this binding may still be in use or cannot be verified.",
            ),
            Self::Unattributed => formatter.write_str(
                "The pane this launch runs in cannot be identified, so its channel enrollment could not be recorded.",
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
    /// The identity this launch runs as.
    pub identity_id: &'a str,
    /// The pane it runs in, persisted by the driver before the foreground starts.
    pub pane: PaneAddress<'a>,
    /// The `tmt run` process that owns this launch; the enrollment is current
    /// only while it lives and the stored binding names it as launch owner.
    pub owner: &'a ProcessIncarnation,
    /// The launch as the user asked for it, resume substitution included. The
    /// driver plans the foreground command from it and never edits it in place.
    pub command: &'a RuntimeCommand,
    /// Exact remembered session selected by the launcher, never inferred from argv.
    pub resume_session: Option<&'a ProviderSessionId>,
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

    /// The owned foreground child was spawned and observed as this exact
    /// incarnation. Called at most once by the launcher, from the observation it
    /// already makes for admission, before admission. An `Err` means the driver
    /// could not record it: the launcher keeps the child and warns, and the
    /// enrollment stays unconfirmed. A driver whose enrollment never heard of a
    /// foreground treats it as unknown, never as ended, and infers an end from
    /// nothing else (EOF, a dead launcher, its own server exiting).
    fn foreground_started(&mut self, _foreground: &ProcessIncarnation) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Retire this launch's enrollment. The launcher calls it only when no child
    /// was ever spawned (the spawn failed without one) or the same child was
    /// confirmed reaped (its wait returned). On any path where the foreground may
    /// still run (a wait error, a panic, an early return after the spawn, launcher
    /// death) it drops the lease instead and the driver's record is left exactly as
    /// it is, so a driver never retires or erases it on its own, in a `Drop` or
    /// otherwise. It must remove only what this launch created and never a
    /// replacement enrollment, even if the launch's own state is already gone.
    fn withdraw(self: Box<Self>);
}

pub trait RuntimeChannel {
    /// Whether Default launcher policy should attempt this driver's channel.
    /// The launcher owns policy; drivers advertise only this default.
    fn enabled_by_default(&self) -> bool {
        false
    }

    /// Verify the provider before anything is bound or spawned. `Ok(Some(text))`
    /// reports an informational advisory and permits enrollment. An unavailable
    /// outcome is `Err`; the driver owns qualification, and the launcher chooses
    /// plain fallback or a strict error;
    /// `Ok(None)` has nothing to say.
    /// The command and launch cwd are provider-neutral input; the driver owns
    /// interpreting flags that change its effective working directory.
    fn preflight(
        &self,
        command: &RuntimeCommand,
        working_directory: &Path,
        directory: &Path,
        deadline: Instant,
    ) -> Result<Option<String>, ChannelError>;

    /// Record this launch's opt-in before the provider starts. The enrollment
    /// is what lets a later send tell "opted in, channel not ready" from "never
    /// opted in".
    fn enroll(&self, plan: &ChannelPlan<'_>) -> Result<Box<dyn ChannelEnrollment>, ChannelError>;

    /// Whether this binding has any enrollment on record, active or not. Read-only
    /// evidence for a send that cannot resolve the binding to an identity: `Ok(false)`
    /// is the only answer that lets such a send fall back to the baseline transport,
    /// and an `Err` means it cannot be told.
    fn enrolled(&self, directory: &Path, binding_id: &str) -> Result<bool, ChannelFault>;

    /// Evidence for a pane, independent of any stored binding (observation may
    /// already have deleted it): whether an enrollment attributed to this exact pane
    /// (the address its `enroll` persisted) is live, or unconfirmed because the
    /// foreground was never recorded. Read-only and bounded by `deadline`.
    /// `enrolled: false` is the only answer that lets the baseline paste through. An
    /// attributed enrollment has ended, and does not block, only when its foreground
    /// was confirmed and every recorded process is conclusively gone. The foreground
    /// is confirmed by the launcher's `foreground_started`, or by a recorded process
    /// that is itself the foreground (the agent the launcher spawned); a helper, a
    /// server or an app-server process the agent started is never a substitute. An
    /// enrollment whose foreground was never confirmed is unknown, not ended: it
    /// stays an `Err` for this pane even after its launcher and every server it
    /// started have disappeared, and only the user's named recovery or an explicit
    /// relaunch in the very pane it names (`enroll`'s takeover rule) clears it.
    /// Evidence about this pane that cannot be told is an `Err`. A record that
    /// cannot be attributed to any pane never blocks an unrelated one: it is listed
    /// in `skipped` instead, unless its file is named for `binding_id`, the binding
    /// the caller is delivering to, which makes it that binding's own invalid
    /// evidence and terminal.
    fn enrolled_in_pane(
        &self,
        directory: &Path,
        pane: &PaneAddress<'_>,
        binding_id: Option<&str>,
        deadline: Instant,
    ) -> Result<PaneEvidence, EvidenceError>;

    /// This driver's enrollment of `binding_id`, with an exact observation of every
    /// process it recorded. Read-only and bounded by `deadline`. `Ok(None)` means
    /// no record; an unreadable record is an `Err` that names it. Every driver that
    /// writes enrollment records implements it and `recover`; the defaults describe
    /// a channel that keeps none.
    fn inspect(
        &self,
        _directory: &Path,
        _binding_id: &str,
        _deadline: Instant,
    ) -> Result<Option<EnrollmentReport>, EvidenceError> {
        Ok(None)
    }

    /// Remove this driver's enrollment of `binding_id` with exactly `generation`,
    /// only when every process it recorded is conclusively gone, under the
    /// driver's own lock and only while the record is unchanged since it was
    /// observed. It never signals a process, never touches a pane or a request,
    /// and never removes a directory recursively. An enrollment whose foreground
    /// was never recorded is removed too: the caller is the user's explicit
    /// request after checking the pane, which is the only proof there is.
    fn recover(
        &self,
        _directory: &Path,
        _binding_id: &str,
        _generation: &str,
        _deadline: Instant,
    ) -> Result<Recovery, RecoveryError> {
        Ok(Recovery::Absent)
    }

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
            ChannelFault::Inactive,
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
        fn preflight(
            &self,
            _: &crate::runtime::RuntimeCommand,
            _: &Path,
            _: &Path,
            _: Instant,
        ) -> Result<Option<String>, ChannelError> {
            Ok(None)
        }

        fn enrolled(&self, _: &Path, _: &str) -> Result<bool, ChannelFault> {
            Ok(false)
        }

        fn enrolled_in_pane(
            &self,
            _: &Path,
            _: &PaneAddress<'_>,
            _: Option<&str>,
            _: Instant,
        ) -> Result<PaneEvidence, EvidenceError> {
            Ok(PaneEvidence::default())
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
        let server = ServerEvidence {
            host: tmt_core::host::HostKind::Tmux,
            server_id: "server".into(),
            socket_path: "/tmp/tmux".into(),
            server_pid: 1,
            server_start_time: "start".into(),
        };
        let user = RuntimeCommand {
            executable: "/bin/agent".into(),
            args: vec!["--model".into(), OsString::from_vec(vec![0xff, b'x'])],
        };
        let plan = |command| ChannelPlan {
            resume_session: None,
            binding_id: "binding",
            identity_id: "identity",
            pane: PaneAddress {
                server: &server,
                pane_id: "%1",
                pane_pid: 2,
            },
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
