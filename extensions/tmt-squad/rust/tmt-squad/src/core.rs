//! The only door to TMT: public `--json` commands and `tmt api`, run through the
//! invoking executable. Squad never opens TMT storage or configuration itself.

use crate::runner::{self, RunError};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(15);
// Above `tmt api`'s own advertised output bound, so its errors stay readable.
const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

/// A squad or passed-through core failure. Core codes are never renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquadError {
    pub code: String,
    pub message: String,
}

impl SquadError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
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
            .map(|executable| Self { executable })
            .ok_or_else(|| {
                unavailable("Could not find the tmt executable; run through `tmt squad`.")
            })
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
        let finished =
            runner::run(&self.executable, argv, input, TIMEOUT, OUTPUT_LIMIT).map_err(|error| {
                unavailable(match error {
                    RunError::Spawn => "Could not start tmt.",
                    RunError::Timeout => "tmt did not finish in time; the outcome is unknown.",
                    RunError::OutputLimit => "tmt output exceeded squad's bound.",
                    RunError::Io => "Could not read tmt output; the outcome is unknown.",
                })
            })?;
        let document: Value = serde_json::from_slice(&finished.stdout)
            .map_err(|_| unavailable("tmt returned no JSON document."))?;
        if finished.success {
            return Ok(document);
        }
        let error = &document["error"];
        match (error["code"].as_str(), error["message"].as_str()) {
            (Some(code), Some(message)) => Err(SquadError::new(code, message)),
            _ => Err(unavailable("tmt failed without a structured error.")),
        }
    }
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
