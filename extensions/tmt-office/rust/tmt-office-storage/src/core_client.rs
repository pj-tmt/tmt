//! Office's only route to core at run time: the invoking `tmt`, through its
//! public JSON commands and `tmt api`. Office never opens the core database.
//!
//! `TMT_EXECUTABLE` selects the executable; a direct run falls back to `tmt`
//! on `PATH`, never to this binary itself.

use serde_json::{Value, json};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_adapters::process::{CommandFailure, CommandRequest, CommandRunner, UnixCommandRunner};

const CORE_DEADLINE: Duration = Duration::from_secs(30);
const CORE_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

/// A failed call. `code` is core's own error code when it answered with one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreCallError {
    pub code: Option<String>,
    pub message: String,
    pub timed_out: bool,
}

impl CoreCallError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
            timed_out: false,
        }
    }
}

impl std::fmt::Display for CoreCallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CoreCallError {}

/// Who a `tmt api` write is attributed to.
#[derive(Debug, Clone, Copy)]
pub enum WriteOriginator<'a> {
    /// An active identity UUID or name.
    Identity(&'a str),
    /// No writer identity, as the CLI without `--identity`.
    Anonymous,
}

#[derive(Debug, Clone)]
pub struct CoreClient {
    tmt: PathBuf,
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

impl CoreClient {
    /// A client for a specific `tmt` executable.
    pub fn with_executable(tmt: PathBuf) -> Self {
        Self { tmt }
    }

    /// Selects the `tmt` without calling it; core initializes its own storage on
    /// first use.
    pub fn discover() -> Result<Self, CoreCallError> {
        let tmt = match std::env::var_os("TMT_EXECUTABLE").map(PathBuf::from) {
            Some(path) if path.is_absolute() && executable(&path) => path,
            Some(_) => {
                return Err(CoreCallError::unavailable(
                    "TMT_EXECUTABLE must select an executable absolute tmt path.",
                ));
            }
            None => std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|directory| directory.join("tmt"))
                .find(|candidate| executable(candidate))
                .ok_or_else(|| CoreCallError::unavailable("Could not find tmt to reach core."))?,
        };
        if let (Ok(selected), Ok(this)) = (
            std::fs::canonicalize(&tmt),
            std::env::current_exe().and_then(std::fs::canonicalize),
        ) && selected == this
        {
            return Err(CoreCallError::unavailable(
                "The selected tmt is this Office binary; refusing recursion.",
            ));
        }
        Ok(Self { tmt })
    }

    /// One JSON command, for example `["room", "list"]`; `--json` is prepended.
    pub fn command(&self, args: &[&str]) -> Result<Value, CoreCallError> {
        let mut all: Vec<OsString> = vec!["--json".into()];
        all.extend(args.iter().map(OsString::from));
        self.run(&all, b"")
    }

    /// One `tmt api` operation. `originator` is required for writes only.
    pub fn api(
        &self,
        operation: &str,
        input: Value,
        originator: Option<WriteOriginator<'_>>,
    ) -> Result<Value, CoreCallError> {
        let mut envelope = json!({"version": 1, "operation": operation, "input": input});
        match originator {
            Some(WriteOriginator::Identity(identity)) => envelope["identity"] = json!(identity),
            Some(WriteOriginator::Anonymous) => envelope["originator"] = json!("anonymous"),
            None => {}
        }
        self.run(&[OsString::from("api")], envelope.to_string().as_bytes())
    }

    fn run(&self, args: &[OsString], input: &[u8]) -> Result<Value, CoreCallError> {
        let output = match UnixCommandRunner.execute(CommandRequest {
            program: self.tmt.as_os_str(),
            args,
            input,
            deadline: Instant::now() + CORE_DEADLINE,
            max_output_bytes: CORE_OUTPUT_LIMIT,
        }) {
            Ok(output) => output.stdout,
            Err(error) => {
                let document = error
                    .output
                    .as_ref()
                    .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());
                let field = |name: &str| {
                    document
                        .as_ref()
                        .and_then(|value| value["error"][name].as_str())
                        .map(str::to_owned)
                };
                return Err(CoreCallError {
                    code: field("code"),
                    message: field("message")
                        .unwrap_or_else(|| "Core could not be reached through tmt.".into()),
                    timed_out: matches!(error.kind, CommandFailure::Timeout),
                });
            }
        };
        serde_json::from_slice(&output)
            .map_err(|_| CoreCallError::unavailable("tmt returned no JSON document."))
    }
}
