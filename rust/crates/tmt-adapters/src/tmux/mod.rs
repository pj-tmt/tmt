//! Concrete tmux evidence/metadata IO. Identity transactions, reconciliation and
//! retirement remain application policy; uncertain observations are not death.

mod binding;
mod caller;
mod evidence;
mod focus;
mod metadata;
mod transport;
pub use binding::{ActionError, BindingSession};
pub use focus::{FocusError, Invoker};
pub use transport::{DeliveryError, DeliveryStage};

#[cfg(test)]
mod evidence_tests;
#[cfg(test)]
mod io_tests;
#[cfg(test)]
mod test_support;

pub use caller::CallerEnvironment;

use crate::process::{
    CommandError, CommandFailure, CommandRequest, CommandRunner, UnixCommandRunner,
};
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use std::{
    ffi::{OsStr, OsString},
    fmt,
    time::{Duration, Instant},
};
use tmt_core::binding::BindingTargetEvidence;
use tmt_core::endpoint::{
    BindingMarker, EndpointProbe, EndpointSnapshot, ServerEvidence, valid_process_id,
    valid_server_id,
};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(1);
const OPERATION_MAX_OUTPUT: usize = 1024 * 1024;
const SERVER_ID_OPTION: &str = "@tmux-team.server-id";
const AGENT_METADATA_OPTION: &str = "@tmux-team.agent";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxFailure {
    Evidence,
    Command,
    MetadataRead,
    MetadataWrite,
    SocketPermission,
}

#[derive(Debug)]
pub struct TmuxError {
    pub kind: TmuxFailure,
    message: &'static str,
    cause: Option<CommandError>,
}

impl TmuxError {
    fn evidence(message: &'static str) -> Self {
        Self {
            kind: TmuxFailure::Evidence,
            message,
            cause: None,
        }
    }

    fn command(kind: TmuxFailure, cause: CommandError) -> Self {
        let kind = if socket_permission_denied(&cause) {
            TmuxFailure::SocketPermission
        } else {
            kind
        };
        let message = match kind {
            TmuxFailure::MetadataRead => "Could not read pane metadata",
            TmuxFailure::MetadataWrite => "Could not write pane metadata",
            TmuxFailure::SocketPermission => "Could not access the tmux socket",
            _ => "Could not execute tmux operation",
        };
        Self {
            kind,
            message,
            cause: Some(cause),
        }
    }

    pub fn cleanup_failed(&self) -> bool {
        self.cause
            .as_ref()
            .is_some_and(CommandError::cleanup_failed)
    }

    pub fn socket_permission_denied(&self) -> bool {
        self.kind == TmuxFailure::SocketPermission
    }

    /// tmux ran and reported failure (for example an unknown target), as
    /// opposed to a timeout, spawn or I/O failure.
    pub fn exited(&self) -> bool {
        self.cause
            .as_ref()
            .is_some_and(|cause| matches!(cause.kind, CommandFailure::Exit { .. }))
    }
}

fn socket_permission_denied(cause: &CommandError) -> bool {
    if !matches!(cause.kind, CommandFailure::Exit { .. }) {
        return false;
    }
    let Some(output) = &cause.output else {
        return false;
    };
    let Ok(stderr) = std::str::from_utf8(&output.stderr) else {
        return false;
    };
    // tmux reports a denied connect through its own exit status, not the OS
    // errno of this parent process. Only its socket-connect diagnostic counts.
    stderr.starts_with("error connecting to ")
        && ["(Permission denied)", "(Operation not permitted)"]
            .iter()
            .any(|ending| stderr.trim_end().ends_with(ending))
}

impl fmt::Display for TmuxError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(output, "{}", self.message)?;
        if let Some(cause) = &self.cause {
            let code = match cause.kind {
                CommandFailure::Timeout => Some("ETIMEDOUT"),
                CommandFailure::OutputLimit => Some("ENOBUFS"),
                _ => match cause.raw_os_error().map(Errno::from_raw) {
                    Some(Errno::EACCES) => Some("EACCES"),
                    Some(Errno::EPERM) => Some("EPERM"),
                    Some(Errno::ENOENT) => Some("ENOENT"),
                    _ => None,
                },
            };
            if let Some(code) = code {
                write!(output, " ({code})")?;
            } else if let CommandFailure::Exit {
                code: Some(code @ 1..=255),
                ..
            } = cause.kind
            {
                write!(output, " (tmux exit {code})")?;
            } else if let CommandFailure::Exit {
                signal: Some(signal),
                ..
            } = cause.kind
            {
                match signal {
                    9 => write!(output, " (SIGKILL)")?,
                    15 => write!(output, " (SIGTERM)")?,
                    _ => {}
                }
            }
            if cause.cleanup_failed() {
                write!(output, " (cleanup failed)")?;
            }
        }
        write!(output, ".")
    }
}

impl std::error::Error for TmuxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|cause| cause as _)
    }
}

#[derive(Default, Clone, Copy)]
pub struct OperationOptions<'a> {
    pub deadline: Option<Instant>,
    pub pane_ids: Option<&'a [String]>,
}

pub struct Tmux<R = UnixCommandRunner> {
    runner: R,
}

impl Default for Tmux {
    fn default() -> Self {
        Self::new(UnixCommandRunner)
    }
}

impl<R: CommandRunner> Tmux<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    fn execute(
        &self,
        args: Vec<String>,
        options: OperationOptions<'_>,
        stage: TmuxFailure,
    ) -> Result<String, TmuxError> {
        let deadline = options.deadline.map_or_else(
            || Instant::now() + OPERATION_TIMEOUT,
            |deadline| deadline.min(Instant::now() + OPERATION_TIMEOUT),
        );
        self.run("tmux", args, deadline, OPERATION_MAX_OUTPUT, stage)
    }

    fn run(
        &self,
        program: &str,
        args: Vec<String>,
        deadline: Instant,
        max_output_bytes: usize,
        stage: TmuxFailure,
    ) -> Result<String, TmuxError> {
        if Instant::now() >= deadline {
            return Err(TmuxError::command(
                stage,
                CommandError::new(CommandFailure::Timeout),
            ));
        }
        let args: Vec<OsString> = args.into_iter().map(OsString::from).collect();
        let output = self
            .runner
            .execute(CommandRequest {
                program: OsStr::new(program),
                args: &args,
                input: &[],
                deadline,
                max_output_bytes,
            })
            .map_err(|cause| TmuxError::command(stage, cause))?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn caller_pane(
        &self,
        environment: &CallerEnvironment,
    ) -> Result<Option<String>, TmuxError> {
        optional_observation(caller::resolve(self, environment))
    }

    fn server_evidence(
        &self,
        socket: Option<&str>,
        expected: Option<&str>,
        options: OperationOptions<'_>,
    ) -> Result<ServerEvidence, TmuxError> {
        let mut args = socket_args(socket);
        args.extend([
            "display-message".into(),
            "-p".into(),
            evidence::server_format(),
        ]);
        evidence::parse_server(
            &self.execute(args, options, TmuxFailure::Command)?,
            expected,
        )
    }

    fn ensure_server_id_on(
        &self,
        socket: Option<&str>,
        options: OperationOptions<'_>,
    ) -> Result<String, TmuxError> {
        let read = || {
            let mut args = socket_args(socket);
            args.extend([
                "show-options".into(),
                "-s".into(),
                "-v".into(),
                SERVER_ID_OPTION.into(),
            ]);
            self.execute(args, options, TmuxFailure::Command)
        };
        let value = match read() {
            Ok(value) => value,
            Err(error) if error.cleanup_failed() || error.socket_permission_denied() => {
                return Err(error);
            }
            Err(_) => {
                let mut args = socket_args(socket);
                args.extend([
                    "set-option".into(),
                    "-s".into(),
                    "-o".into(),
                    SERVER_ID_OPTION.into(),
                    uuid::Uuid::new_v4().to_string(),
                ]);
                self.execute(args, options, TmuxFailure::Command)?;
                read()?
            }
        };
        let value = value.trim();
        if !valid_server_id(value) {
            return Err(TmuxError::evidence("tmux server identity is unavailable"));
        }
        Ok(value.into())
    }

    fn ensure_server_id(&self, options: OperationOptions<'_>) -> Result<String, TmuxError> {
        self.ensure_server_id_on(None, options)
    }

    /// Resolve the explicit mark on the invocation-selected server once. The
    /// returned endpoint evidence is independent of later focus or mark moves.
    pub fn marked_pane(
        &self,
        environment: &CallerEnvironment,
        options: OperationOptions<'_>,
    ) -> Result<Option<BindingTargetEvidence>, TmuxError> {
        let socket = environment.selected_server_socket()?;
        let expected = self.ensure_server_id_on(socket, options)?;
        let mut args = socket_args(socket);
        args.extend([
            "list-panes".into(),
            "-a".into(),
            "-f".into(),
            "#{pane_marked}".into(),
            "-F".into(),
            evidence::endpoint_format(),
        ]);
        let output = self.execute(args, options, TmuxFailure::Command)?;
        if output.trim().is_empty() {
            return Ok(None);
        }
        let snapshot = evidence::parse_snapshot(&output, Some(&expected))?;
        if snapshot.panes.len() != 1 {
            return Err(TmuxError::evidence(
                "tmux marked pane evidence is ambiguous",
            ));
        }
        let pane = &snapshot.panes[0];
        Ok(Some(BindingTargetEvidence {
            server: snapshot.server,
            pane_id: pane.id.clone(),
            pane_pid: pane.pane_pid,
        }))
    }

    /// Observe existing endpoint evidence without initializing server metadata.
    /// A server without a valid TMT marker is unavailable, not silently adopted.
    pub fn observe_snapshot(
        &self,
        options: OperationOptions<'_>,
    ) -> Result<EndpointSnapshot, TmuxError> {
        let scope = evidence::scoped_ids(options.pane_ids)?;
        let output = self.execute(
            list_args(None, scope.as_deref()),
            options,
            TmuxFailure::Command,
        )?;
        evidence::parse_snapshot(&output, None)
    }

    pub fn snapshot(&self, options: OperationOptions<'_>) -> Result<EndpointSnapshot, TmuxError> {
        let scope = evidence::scoped_ids(options.pane_ids)?;
        let expected = self.ensure_server_id(options)?;
        if scope.as_ref().is_some_and(Vec::is_empty) {
            return Ok(EndpointSnapshot {
                server: self.server_evidence(None, Some(&expected), options)?,
                panes: Vec::new(),
            });
        }
        let output = self.execute(
            list_args(None, scope.as_deref()),
            options,
            TmuxFailure::Command,
        )?;
        if !output.trim().is_empty() {
            return evidence::parse_snapshot(&output, Some(&expected));
        }
        if scope.is_none() {
            return Err(TmuxError::evidence("tmux endpoint snapshot is empty"));
        }
        Ok(EndpointSnapshot {
            server: self.server_evidence(None, Some(&expected), options)?,
            panes: Vec::new(),
        })
    }

    pub fn probe(
        &self,
        socket: &str,
        recorded_pid: u64,
        options: OperationOptions<'_>,
    ) -> Result<EndpointProbe, TmuxError> {
        if socket.is_empty() || !valid_process_id(recorded_pid) {
            return Ok(EndpointProbe::Unknown);
        }
        let Ok(scope) = evidence::scoped_ids(options.pane_ids) else {
            return Ok(EndpointProbe::Unknown);
        };
        if scope.as_ref().is_some_and(Vec::is_empty) {
            return match self.server_evidence(Some(socket), None, options) {
                Ok(server) if server.socket_path == socket => {
                    Ok(EndpointProbe::Live(EndpointSnapshot {
                        server,
                        panes: Vec::new(),
                    }))
                }
                Err(error) if error.cleanup_failed() || error.socket_permission_denied() => {
                    Err(error)
                }
                Err(_) => Ok(probe_death(recorded_pid)),
                _ => Ok(EndpointProbe::Unknown),
            };
        }
        let output = match self.execute(
            list_args(Some(socket), scope.as_deref()),
            options,
            TmuxFailure::Command,
        ) {
            Ok(output) => output,
            Err(error) if error.cleanup_failed() || error.socket_permission_denied() => {
                return Err(error);
            }
            Err(_) => return Ok(probe_death(recorded_pid)),
        };
        let observed = if !output.trim().is_empty() {
            evidence::parse_snapshot(&output, None)
        } else if scope.is_some() {
            self.server_evidence(Some(socket), None, options)
                .map(|server| EndpointSnapshot {
                    server,
                    panes: Vec::new(),
                })
        } else {
            Err(TmuxError::evidence("tmux endpoint snapshot is empty"))
        };
        match observed {
            Ok(snapshot) if snapshot.server.socket_path == socket => {
                Ok(EndpointProbe::Live(snapshot))
            }
            Err(error) if error.cleanup_failed() || error.socket_permission_denied() => Err(error),
            // A fresh server at a reused socket has no TMT server marker yet.
            // Malformed output alone is not death, but ESRCH for the recorded
            // process is independent, conclusive evidence. Never initialize the
            // foreign server merely to reconcile the old binding.
            Err(_) => Ok(probe_death(recorded_pid)),
            _ => Ok(EndpointProbe::Unknown),
        }
    }

    pub fn resolve_target(
        &self,
        target: &str,
        options: OperationOptions<'_>,
    ) -> Result<Option<String>, TmuxError> {
        let output = optional_observation(self.execute(
            vec![
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                target.into(),
                "#{pane_id}".into(),
            ],
            options,
            TmuxFailure::Command,
        ))?;
        let Some(output) = output else {
            return Ok(None);
        };
        let id = output.trim();
        Ok(tmt_core::endpoint::valid_pane_id(id).then(|| id.into()))
    }

    fn read_metadata(
        &self,
        socket: Option<&str>,
        pane: &str,
        options: OperationOptions<'_>,
    ) -> Result<serde_json::Value, TmuxError> {
        let mut args = socket_args(socket);
        args.extend([
            "show-options".into(),
            "-q".into(),
            "-p".into(),
            "-t".into(),
            pane.into(),
            "-v".into(),
            AGENT_METADATA_OPTION.into(),
        ]);
        let output = self.execute(args, options, TmuxFailure::MetadataRead)?;
        Ok(metadata::decode(&output))
    }

    fn write_metadata(
        &self,
        socket: Option<&str>,
        pane: &str,
        document: &serde_json::Value,
        options: OperationOptions<'_>,
    ) -> Result<(), TmuxError> {
        let mut args = socket_args(socket);
        args.extend(["set-option".into(), "-p".into()]);
        if !metadata::has_fields(document) {
            args.push("-u".into());
        }
        args.extend(["-t".into(), pane.into(), AGENT_METADATA_OPTION.into()]);
        if metadata::has_fields(document) {
            args.push(document.to_string());
        }
        self.execute(args, options, TmuxFailure::MetadataWrite)
            .map(|_| ())
    }

    pub fn set_marker(
        &self,
        pane: &str,
        marker: &BindingMarker,
        options: OperationOptions<'_>,
    ) -> Result<(), TmuxError> {
        self.set_marker_on(None, pane, marker, options)
    }

    fn set_marker_on(
        &self,
        socket: Option<&str>,
        pane: &str,
        marker: &BindingMarker,
        options: OperationOptions<'_>,
    ) -> Result<(), TmuxError> {
        let mut document = self.read_metadata(socket, pane, options)?;
        metadata::replace(&mut document, marker);
        self.write_metadata(socket, pane, &document, options)
    }

    pub fn clear_marker(
        &self,
        pane: &str,
        binding_id: Option<&str>,
        options: OperationOptions<'_>,
    ) -> Result<bool, TmuxError> {
        self.clear_marker_on(None, pane, binding_id, options)
    }

    fn clear_marker_on(
        &self,
        socket: Option<&str>,
        pane: &str,
        binding_id: Option<&str>,
        options: OperationOptions<'_>,
    ) -> Result<bool, TmuxError> {
        let mut document = self.read_metadata(socket, pane, options)?;
        if !metadata::clear(&mut document, binding_id) {
            return Ok(false);
        }
        self.write_metadata(socket, pane, &document, options)?;
        Ok(true)
    }
}

/// Expected unavailable evidence is best-effort; failed resource cleanup is
/// exceptional and must reach the invocation owner, never become "not found".
fn optional_observation<T>(result: Result<T, TmuxError>) -> Result<Option<T>, TmuxError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.cleanup_failed() || error.socket_permission_denied() => Err(error),
        Err(_) => Ok(None),
    }
}

fn probe_death(recorded_pid: u64) -> EndpointProbe {
    let Ok(pid) = i32::try_from(recorded_pid) else {
        return EndpointProbe::Unknown;
    };
    match kill(Pid::from_raw(pid), None) {
        Err(Errno::ESRCH) => EndpointProbe::Dead,
        _ => EndpointProbe::Unknown,
    }
}

fn socket_args(socket: Option<&str>) -> Vec<String> {
    socket.map_or_else(Vec::new, |socket| vec!["-S".into(), socket.into()])
}

fn list_args(socket: Option<&str>, scope: Option<&[&str]>) -> Vec<String> {
    let mut args = socket_args(socket);
    args.extend(["list-panes".into(), "-a".into()]);
    if let Some(ids) = scope.filter(|ids| !ids.is_empty()) {
        args.extend(["-f".into(), evidence::pane_filter(ids)]);
    }
    args.extend(["-F".into(), evidence::endpoint_format()]);
    args
}
