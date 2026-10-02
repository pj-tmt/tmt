//! Herdr's documented CLI (`herdr <group> <command>`, JSON out), on a named
//! socket or Herdr's own default. Success is a JSON document on stdout; an
//! API failure is `{error:{code,message}}` on stderr with exit status 1.

use crate::run::{Output, RunError, Runner};
use serde_json::Value;
use std::time::Instant;
use tmt_driver_protocol::{DriverError, ErrorCode};

/// The first Herdr release with every API the driver uses.
const FLOOR: (u64, u64, u64) = (0, 9, 1);

/// Why a Herdr call gave no answer, mapped to the protocol's codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HerdrError {
    /// Herdr answered with its own error code, such as `pane_not_found`.
    Api { code: String, message: String },
    /// The answer was not the document Herdr documents.
    Malformed(&'static str),
    /// `herdr` could not run or did not finish.
    Process(RunError),
    /// The running server is older than the floor.
    Version(String),
}

impl HerdrError {
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Api { code, .. } => Some(code),
            _ => None,
        }
    }

    pub fn into_driver(self) -> DriverError {
        match self {
            Self::Api { code, message } => {
                let protocol = match code.as_str() {
                    "pane_not_found" => ErrorCode::NotFound,
                    "server_not_running" => ErrorCode::Unavailable,
                    _ => ErrorCode::Failed,
                };
                DriverError::new(protocol, format!("Herdr: {message} ({code})"))
            }
            Self::Malformed(reason) => DriverError::new(ErrorCode::Failed, reason),
            Self::Process(RunError::NotStarted) => {
                DriverError::new(ErrorCode::Unavailable, "herdr could not be run")
            }
            Self::Process(RunError::Unfinished) => {
                DriverError::new(ErrorCode::Failed, "herdr did not finish in time")
            }
            Self::Version(found) => DriverError::new(
                ErrorCode::Unavailable,
                format!(
                    "Herdr {}.{}.{} or later is needed (found {found})",
                    FLOOR.0, FLOOR.1, FLOOR.2
                ),
            ),
        }
    }
}

pub struct Herdr<R> {
    runner: R,
}

impl<R: Runner> Herdr<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }

    fn execute(
        &self,
        socket: Option<&str>,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Vec<u8>, HerdrError> {
        let env: Vec<(&str, &str)> = socket
            .map(|socket| ("HERDR_SOCKET_PATH", socket))
            .into_iter()
            .collect();
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let Output {
            code,
            stdout,
            stderr,
        } = self
            .runner
            .run("herdr", &env, &args, deadline)
            .map_err(HerdrError::Process)?;
        match code {
            Some(0) => Ok(stdout),
            Some(1) => Err(api_error(&stderr).unwrap_or(HerdrError::Malformed(
                "Herdr failed without a documented error",
            ))),
            Some(127) => Err(HerdrError::Process(RunError::NotStarted)),
            _ => Err(HerdrError::Malformed("Herdr exited unexpectedly")),
        }
    }

    fn document(
        &self,
        socket: Option<&str>,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Value, HerdrError> {
        let stdout = self.execute(socket, args, deadline)?;
        serde_json::from_slice(&stdout)
            .map_err(|_| HerdrError::Malformed("Herdr returned malformed JSON"))
    }

    /// An API call's `result`.
    pub fn call(
        &self,
        socket: &str,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Value, HerdrError> {
        let mut document = self.document(Some(socket), args, deadline)?;
        match document.get_mut("result") {
            Some(result) => Ok(result.take()),
            None => Err(HerdrError::Malformed("Herdr returned no result")),
        }
    }

    /// A call that prints plain text (`pane read`).
    pub fn text(
        &self,
        socket: &str,
        args: &[&str],
        deadline: Instant,
    ) -> Result<String, HerdrError> {
        self.execute(Some(socket), args, deadline)
            .map(|stdout| String::from_utf8_lossy(&stdout).into_owned())
    }

    /// A call whose success prints nothing (`pane report-metadata`).
    pub fn act(&self, socket: &str, args: &[&str], deadline: Instant) -> Result<(), HerdrError> {
        self.execute(Some(socket), args, deadline).map(|_| ())
    }

    /// The running server's socket on `socket` (or Herdr's default), at the
    /// floor; `None` when no server runs there.
    pub fn running(
        &self,
        socket: Option<&str>,
        deadline: Instant,
    ) -> Result<Option<String>, HerdrError> {
        let status = self.document(socket, &["status", "server", "--json"], deadline)?;
        if status["running"].as_bool() != Some(true) {
            return Ok(None);
        }
        let socket = status["socket"]
            .as_str()
            .filter(|socket| socket.starts_with('/'))
            .ok_or(HerdrError::Malformed("Herdr did not report its socket"))?;
        check_floor(status["version"].as_str())?;
        Ok(Some(socket.to_owned()))
    }
}

fn api_error(stderr: &[u8]) -> Option<HerdrError> {
    let document: Value = serde_json::from_slice(stderr).ok()?;
    let code = document["error"]["code"].as_str()?;
    let valid = !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_' || byte.is_ascii_digit());
    valid.then(|| HerdrError::Api {
        code: code.to_owned(),
        message: document["error"]["message"]
            .as_str()
            .filter(|message| message.len() <= 256 && !message.chars().any(char::is_control))
            .unwrap_or("unknown error")
            .to_owned(),
    })
}

/// A semantic version at or above the floor; a pre-release of the floor
/// orders below it, and a newer release is trusted.
fn check_floor(version: Option<&str>) -> Result<(), HerdrError> {
    let floor = semver::Version::new(FLOOR.0, FLOOR.1, FLOOR.2);
    match version.and_then(|version| semver::Version::parse(version).ok()) {
        Some(parsed) if parsed >= floor => Ok(()),
        _ => Err(HerdrError::Version(
            version
                .filter(|version| version.len() <= 32)
                .unwrap_or("an unknown version")
                .to_owned(),
        )),
    }
}
