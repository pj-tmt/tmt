//! Herdr (#479), the second terminal host. TMT reaches it only through its
//! documented CLI (`herdr <group> <command>`, JSON out) under the bounded
//! process owner, like tmux. Pane identity is Herdr's terminal ID, which
//! follows a pane through moves; a server incarnation is the server process
//! (the parent of every pane shell) and its start time, and TMT's own UUID for
//! it comes from the injected [`crate::host::HostServerIds`] port before any
//! binding transaction. The marker is display-only pane tokens under source
//! `tmt`, evidence only when its IDs match storage.

mod caller;
mod evidence;
mod marker;
mod session;
#[cfg(test)]
mod tests;

pub use session::Session;

use crate::process::{CommandError, CommandFailure, CommandRequest, CommandRunner};
use serde_json::Value;
use std::{
    cell::OnceCell,
    ffi::OsString,
    fmt,
    time::{Duration, Instant},
};
use tmt_core::endpoint::ServerEvidence;

const OPERATION_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_OUTPUT: usize = 1024 * 1024;
/// The first release with every API TMT uses (design §1.1, decision 3).
const FLOOR: (u64, u64, u64) = (0, 9, 1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HerdrFailure {
    /// Missing, malformed or inconsistent evidence.
    Evidence,
    /// The CLI could not run or did not finish.
    Command,
    /// Herdr answered with an error code.
    Api,
    /// The running server is older than TMT's floor.
    Version,
    /// TMT could not record the server incarnation.
    ServerIds,
}

#[derive(Debug)]
pub struct HerdrError {
    pub kind: HerdrFailure,
    message: String,
    code: Option<String>,
    cause: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl HerdrError {
    fn evidence(message: impl Into<String>) -> Self {
        Self {
            kind: HerdrFailure::Evidence,
            message: message.into(),
            code: None,
            cause: None,
        }
    }

    fn command(cause: CommandError) -> Self {
        Self {
            kind: HerdrFailure::Command,
            message: "Could not execute Herdr operation".into(),
            code: None,
            cause: Some(Box::new(cause)),
        }
    }

    fn api(code: String, message: &str) -> Self {
        Self {
            kind: HerdrFailure::Api,
            message: format!("Herdr refused the operation: {message}"),
            code: Some(code),
            cause: None,
        }
    }

    pub(crate) fn server_ids(cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            kind: HerdrFailure::ServerIds,
            message: "Could not record the Herdr server".into(),
            code: None,
            cause: Some(Box::new(cause)),
        }
    }

    pub(crate) fn unsupported(what: &str) -> Self {
        Self::evidence(format!("{what} is not supported yet"))
    }

    /// Herdr's own error code, such as `pane_not_found`.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    pub fn cleanup_failed(&self) -> bool {
        self.cause
            .as_deref()
            .and_then(|cause| cause.downcast_ref::<CommandError>())
            .is_some_and(CommandError::cleanup_failed)
    }
}

impl fmt::Display for HerdrError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(output, "{}", self.message)?;
        if let Some(code) = &self.code {
            write!(output, " ({code})")?;
        } else if let Some(cause) = self
            .cause
            .as_deref()
            .and_then(|cause| cause.downcast_ref::<CommandError>())
        {
            match cause.kind {
                CommandFailure::Timeout => write!(output, " (ETIMEDOUT)")?,
                CommandFailure::OutputLimit => write!(output, " (ENOBUFS)")?,
                CommandFailure::Spawn => write!(output, " (herdr not found)")?,
                _ => {}
            }
            if cause.cleanup_failed() {
                write!(output, " (cleanup failed)")?;
            }
        }
        write!(output, ".")
    }
}

impl std::error::Error for HerdrError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_deref().map(|cause| cause as _)
    }
}

/// `herdr status server --json`, the one call that also resolves the socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Status {
    pub socket: String,
    pub running: bool,
    pub version: Option<String>,
}

pub struct Herdr<R> {
    runner: R,
    /// The socket a caller's environment selects; `None` lets Herdr resolve
    /// its default, as a user's own `herdr` command would.
    socket: Option<String>,
    /// This handle's server, resolved once before any binding transaction.
    server: OnceCell<ServerEvidence>,
}

impl<R: CommandRunner> Herdr<R> {
    pub fn new(runner: R, socket: Option<String>) -> Self {
        Self {
            runner,
            socket,
            server: OnceCell::new(),
        }
    }

    pub(crate) fn resolved(&self) -> Option<&ServerEvidence> {
        self.server.get()
    }

    /// The first resolution stands for the handle's lifetime.
    pub(crate) fn set_resolved(&self, server: ServerEvidence) {
        let _ = self.server.set(server);
    }

    pub(crate) fn runner(&self) -> &R {
        &self.runner
    }

    fn deadline(deadline: Option<Instant>) -> Instant {
        let bound = Instant::now() + OPERATION_TIMEOUT;
        deadline.map_or(bound, |deadline| deadline.min(bound))
    }

    /// One CLI call on `socket` (or Herdr's own resolution). Success is a JSON
    /// document on stdout; failure is `{error:{code,message}}` on stderr.
    fn run(
        &self,
        socket: Option<&str>,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Value, HerdrError> {
        let stdout = self.execute(socket, args, deadline)?;
        serde_json::from_slice(&stdout)
            .map_err(|_| HerdrError::evidence("Herdr returned malformed JSON"))
    }

    /// A call whose success prints nothing (`pane report-metadata`).
    fn act(&self, socket: &str, args: &[&str], deadline: Instant) -> Result<(), HerdrError> {
        self.execute(Some(socket), args, deadline).map(|_| ())
    }

    fn execute(
        &self,
        socket: Option<&str>,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Vec<u8>, HerdrError> {
        if Instant::now() >= deadline {
            return Err(HerdrError::command(CommandError::new(
                CommandFailure::Timeout,
            )));
        }
        let mut argv: Vec<OsString> = Vec::new();
        if let Some(socket) = socket {
            argv.push(format!("HERDR_SOCKET_PATH={socket}").into());
        }
        argv.push("herdr".into());
        argv.extend(args.iter().map(OsString::from));
        let result = self.runner.execute(CommandRequest {
            program: "/usr/bin/env".as_ref(),
            args: &argv,
            input: &[],
            deadline,
            max_output_bytes: MAX_OUTPUT,
        });
        match result {
            Ok(output) => Ok(output.stdout),
            Err(error) => Err(api_error(&error).unwrap_or_else(|| HerdrError::command(error))),
        }
    }

    /// An API call's `result`.
    fn call(
        &self,
        socket: Option<&str>,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Value, HerdrError> {
        let mut document = self.run(socket, args, deadline)?;
        match document.get_mut("result") {
            Some(result) => Ok(result.take()),
            None => Err(HerdrError::evidence("Herdr returned no result")),
        }
    }

    pub(crate) fn status(
        &self,
        socket: Option<&str>,
        deadline: Instant,
    ) -> Result<Status, HerdrError> {
        let value = self.run(socket, &["status", "server", "--json"], deadline)?;
        let socket = value["socket"]
            .as_str()
            .filter(|socket| socket.starts_with('/'))
            .ok_or_else(|| HerdrError::evidence("Herdr did not report its socket"))?;
        Ok(Status {
            socket: socket.into(),
            running: value["running"].as_bool() == Some(true),
            version: value["version"].as_str().map(str::to_owned),
        })
    }

    /// The running server on `socket` (or this handle's), at TMT's floor.
    pub(crate) fn server_socket(
        &self,
        socket: Option<&str>,
        deadline: Instant,
    ) -> Result<String, HerdrError> {
        let status = self.status(socket.or(self.socket.as_deref()), deadline)?;
        if !status.running {
            return Err(HerdrError::api(
                "server_not_running".into(),
                "no Herdr server is running",
            ));
        }
        check_floor(status.version.as_deref())?;
        Ok(status.socket)
    }
}

impl<R: CommandRunner> Herdr<R> {
    /// The server incarnation on `socket`, or `None` when it has no pane.
    pub(crate) fn incarnation(
        &self,
        socket: &str,
        deadline: Instant,
    ) -> Result<Option<evidence::Incarnation>, HerdrError> {
        Ok(self
            .observe(socket, Some(&[]), deadline)?
            .map(|observed| observed.incarnation))
    }

    /// A public `wN:pM` to its terminal ID on the default (or caller's)
    /// server; `None` when no such pane exists or Herdr is not reachable.
    pub(crate) fn resolve_target(
        &self,
        target: &str,
        deadline: Option<Instant>,
    ) -> Result<Option<String>, HerdrError> {
        let deadline = Self::deadline(deadline);
        let found = self
            .server_socket(None, deadline)
            .and_then(|socket| self.call(Some(&socket), &["pane", "get", target], deadline));
        match found {
            Ok(result) => Ok(result["pane"]["terminal_id"]
                .as_str()
                .filter(|id| tmt_core::host::HostKind::Herdr.is_pane_id(id))
                .map(str::to_owned)),
            Err(error) if error.cleanup_failed() => Err(error),
            Err(_) => Ok(None),
        }
    }
}

fn api_error(error: &CommandError) -> Option<HerdrError> {
    if !matches!(error.kind, CommandFailure::Exit { code: Some(1), .. }) || error.cleanup_failed() {
        return None;
    }
    let output = error.output.as_ref()?;
    let document: Value = serde_json::from_slice(&output.stderr).ok()?;
    let code = document["error"]["code"].as_str()?;
    let valid = !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_' || byte.is_ascii_digit());
    valid.then(|| {
        HerdrError::api(
            code.into(),
            document["error"]["message"]
                .as_str()
                .filter(|message| message.len() <= 512)
                .unwrap_or("unknown error"),
        )
    })
}

/// `major.minor.patch`, optionally with a pre-release or build suffix. A
/// newer protocol is trusted (decision 3): only older releases are refused.
fn check_floor(version: Option<&str>) -> Result<(), HerdrError> {
    let parsed = version.and_then(|version| {
        let core = version.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        let parsed = (parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(parsed)
    });
    match parsed {
        Some(parsed) if parsed >= FLOOR => Ok(()),
        _ => Err(HerdrError {
            kind: HerdrFailure::Version,
            message: format!(
                "TMT needs Herdr {}.{}.{} or later (found {})",
                FLOOR.0,
                FLOOR.1,
                FLOOR.2,
                version
                    .filter(|version| version.len() <= 32)
                    .unwrap_or("an unknown version")
            ),
            code: None,
            cause: None,
        }),
    }
}
