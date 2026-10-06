//! The only door to TMT: public `--json` commands and `tmt api`, run through the
//! invoking executable. Squad never opens TMT storage or configuration itself.

use crate::runner::{self, RunError};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(15);
// Above `tmt api`'s own advertised output bound, so its errors stay readable.
const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

/// A squad or passed-through core failure. Core codes are never renamed.
/// `message` is the whole text `--json` reports; human output shows it as
/// `error:`, or as `error:` plus `hint:` when the failure names a next step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquadError {
    pub code: String,
    pub message: String,
    /// Transaction conflict snapshot supplied by the public metadata API.
    pub current: Option<Box<Value>>,
    /// Where `message` splits into what failed and the next step.
    hint: Option<(usize, usize)>,
}

impl SquadError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
            current: None,
        }
    }

    /// `message` is `what`, `separator` and `hint` joined, exactly as
    /// `--json` has always reported it; human output shows the two parts.
    pub fn hinted(code: &str, what: &str, separator: &str, hint: &str) -> Self {
        Self {
            code: code.into(),
            message: format!("{what}{separator}{hint}"),
            hint: Some((what.len(), what.len() + separator.len())),
            current: None,
        }
    }

    /// What failed, and the next step when there is one.
    pub fn human(&self) -> (&str, Option<&str>) {
        match self.hint {
            Some((what, hint)) => (&self.message[..what], Some(&self.message[hint..])),
            None => (&self.message, None),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({"error": {"code": self.code, "message": self.message}})
    }
}

impl fmt::Display for SquadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({})", self.message, self.code)
    }
}

fn unavailable(message: &str) -> SquadError {
    SquadError::new("SQUAD_CORE_UNAVAILABLE", message)
}

#[derive(Clone)]
pub struct Core {
    executable: PathBuf,
    cancellation: Option<runner::Cancellation>,
    deadline: Option<Instant>,
}

impl Core {
    /// Extension dispatch supplies `TMT_EXECUTABLE`; a direct run falls back
    /// to the first executable `tmt` on PATH.
    pub fn discover() -> Result<Self, SquadError> {
        let supplied = std::env::var_os("TMT_EXECUTABLE").map(PathBuf::from);
        let executable = match supplied {
            Some(path) if path.is_absolute() => Some(path),
            Some(_) => return Err(unavailable("TMT_EXECUTABLE must be an absolute path.")),
            None => std::env::var_os("PATH").and_then(|search| {
                std::env::split_paths(&search)
                    .map(|directory| directory.join("tmt"))
                    .find(|candidate| executable(candidate))
            }),
        };
        executable
            .map(|executable| Self {
                executable,
                cancellation: None,
                deadline: None,
            })
            .ok_or_else(|| {
                unavailable("Could not find the tmt executable; run through `tmt squad`.")
            })
    }

    /// A given executable, for tests that stand a script in for tmt.
    #[cfg(test)]
    pub fn at(executable: PathBuf) -> Self {
        Self {
            executable,
            cancellation: None,
            deadline: None,
        }
    }

    pub fn cancellable(&self, cancellation: runner::Cancellation) -> Self {
        Self {
            cancellation: Some(cancellation),
            ..self.clone()
        }
    }

    /// Context calls share their invocation deadline and the host-owned group.
    pub fn until(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// The tmt this invocation reaches, as extension dispatch supplied it.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Runs `tmt <args> --json` and returns its document, or core's own error.
    pub fn json(&self, args: &[&str]) -> Result<Value, SquadError> {
        let mut argv: Vec<OsString> = args.iter().map(OsString::from).collect();
        argv.push("--json".into());
        self.call(&argv, b"")
    }

    /// Like [`Core::json`], with operands after `--` so text that starts with
    /// `-` is never read as an option.
    pub fn json_with_operands(
        &self,
        args: &[&str],
        operands: &[&str],
    ) -> Result<Value, SquadError> {
        let mut argv: Vec<OsString> = args.iter().map(OsString::from).collect();
        argv.extend(["--json".into(), "--".into()]);
        argv.extend(operands.iter().map(OsString::from));
        self.call(&argv, b"")
    }

    /// One versioned `tmt api` request.
    pub fn api(&self, operation: &str, input: Value) -> Result<Value, SquadError> {
        let request = json!({"version": 1, "operation": operation, "input": input});
        self.call(&["api".into()], request.to_string().as_bytes())
    }

    fn call(&self, argv: &[OsString], input: &[u8]) -> Result<Value, SquadError> {
        let result = match self.deadline {
            Some(deadline) => {
                runner::run_inherited(&self.executable, argv, input, deadline, OUTPUT_LIMIT)
            }
            None => runner::run_cancellable(
                &self.executable,
                argv,
                input,
                TIMEOUT,
                OUTPUT_LIMIT,
                self.cancellation.as_ref(),
            ),
        };
        let finished = result.map_err(|error| {
            unavailable(match error {
                RunError::Spawn => "Could not start tmt.",
                RunError::Timeout => "tmt did not finish in time; the outcome is unknown.",
                RunError::OutputLimit => "tmt output exceeded squad's bound.",
                RunError::Io => "Could not read tmt output; the outcome is unknown.",
                RunError::Cancelled => "The board load was superseded.",
            })
        })?;
        let document: Value = serde_json::from_slice(&finished.stdout)
            .map_err(|_| unavailable("tmt returned no JSON document."))?;
        if finished.success {
            return Ok(document);
        }
        let error = &document["error"];
        match (error["code"].as_str(), error["message"].as_str()) {
            (Some(code), Some(message)) => {
                let mut failure = SquadError::new(code, message);
                failure.current = error.get("current").cloned().map(Box::new);
                Err(failure)
            }
            _ => Err(unavailable("tmt failed without a structured error.")),
        }
    }

    /// A write explicitly attributes its originator, including anonymous clocks.
    pub fn api_write(
        &self,
        operation: &str,
        input: Value,
        identity: Option<&str>,
    ) -> Result<Value, SquadError> {
        let mut request = json!({"version": 1, "operation": operation, "input": input});
        if let Some(identity) = identity {
            request["identity"] = json!(identity);
        } else {
            request["originator"] = json!("anonymous");
        }
        self.call(&["api".into()], request.to_string().as_bytes())
    }
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
