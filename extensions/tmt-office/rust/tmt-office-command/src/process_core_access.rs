//! Direct Office lookup through the public core CLI JSON contract.

use crate::core_access::{CoreAccess, OfficeIdentity, RoomHistory};
use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_adapters::process::{CommandFailure, CommandRequest, CommandRunner, UnixCommandRunner};
use tmt_command_output::Failure;

const CORE_DEADLINE: Duration = Duration::from_secs(30);
const CORE_OUTPUT_LIMIT: usize = 65_536;

pub struct ProcessCoreAccess {
    executable: PathBuf,
}

fn unavailable(message: impl Into<String>) -> Failure {
    Failure::new("OFFICE_CORE_UNAVAILABLE", message, 1)
}

fn executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

impl ProcessCoreAccess {
    pub fn discover() -> Result<Self, Failure> {
        let selected = if let Some(configured) = std::env::var_os("TMT_EXECUTABLE") {
            let path = PathBuf::from(configured);
            if !path.is_absolute() || !executable(&path) {
                return Err(unavailable(
                    "TMT_EXECUTABLE must select an executable absolute tmt path.",
                ));
            }
            path
        } else {
            let path = std::env::var_os("PATH").unwrap_or_default();
            std::env::split_paths(&path)
                .map(|directory| directory.join("tmt"))
                .find(|candidate| executable(candidate))
                .ok_or_else(|| {
                    unavailable("Could not find tmt on PATH for direct Office commands.")
                })?
        };
        let selected = if selected.is_absolute() {
            selected
        } else {
            std::env::current_dir()
                .map_err(|error| unavailable(error.to_string()))?
                .join(selected)
        };
        if let (Ok(selected_file), Ok(self_file)) = (
            fs::canonicalize(&selected),
            std::env::current_exe().and_then(fs::canonicalize),
        ) && selected_file == self_file
        {
            return Err(unavailable(
                "The selected tmt executable is this Office companion; refusing recursive dispatch.",
            ));
        }
        Ok(Self {
            executable: selected,
        })
    }

    fn invoke(&self, arguments: &[&str]) -> Result<serde_json::Value, Failure> {
        let args: Vec<OsString> = std::iter::once(OsString::from("--json"))
            .chain(arguments.iter().map(OsString::from))
            .collect();
        let result = UnixCommandRunner.execute(CommandRequest {
            program: self.executable.as_os_str(),
            args: &args,
            input: b"",
            deadline: Instant::now() + CORE_DEADLINE,
            max_output_bytes: CORE_OUTPUT_LIMIT,
        });
        let output = match result {
            Ok(output) => output,
            Err(error) => {
                if let CommandFailure::Exit {
                    code: Some(status), ..
                } = error.kind
                    && let Some(output) = &error.output
                    && let Some(failure) = child_failure(&output.stdout, status)
                {
                    return Err(failure);
                }
                return Err(
                    unavailable("Could not complete a bounded core lookup.").caused_by(error)
                );
            }
        };
        serde_json::from_slice(&output.stdout)
            .map_err(|error| unavailable("Core lookup returned invalid JSON.").caused_by(error))
    }
}

fn child_failure(stdout: &[u8], status: u32) -> Option<Failure> {
    let value: serde_json::Value = serde_json::from_slice(stdout).ok()?;
    let error = value.get("error")?.as_object()?;
    let code = error.get("code")?.as_str()?;
    let message = error.get("message")?.as_str()?;
    if code.is_empty()
        || code.len() > 80
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        || message.len() > CORE_OUTPUT_LIMIT
    {
        return None;
    }
    let status = u8::try_from(status).ok().filter(|value| *value != 0)?;
    let mut failure = Failure::new(code, message, status);
    if let Some(suggestion) = error.get("suggestion").and_then(serde_json::Value::as_str) {
        if suggestion.len() > CORE_OUTPUT_LIMIT {
            return None;
        }
        failure = failure.suggestion(suggestion.to_owned());
    }
    Some(failure)
}

fn identity(value: &serde_json::Value, nested: bool) -> Result<OfficeIdentity, Failure> {
    let object = if nested {
        value.get("identity")
    } else {
        Some(value)
    }
    .ok_or_else(|| unavailable("Core identity lookup returned an incomplete document."))?;
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| unavailable("Core identity lookup omitted its UUID."))?;
    let name = object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| unavailable("Core identity lookup omitted its name."))?;
    Ok(OfficeIdentity {
        id: id.into(),
        name: name.into(),
    })
}

impl CoreAccess for ProcessCoreAccess {
    fn identity(&self, selector: Option<&str>) -> Result<OfficeIdentity, Failure> {
        if let Some(selector) = selector {
            let value = self
                .invoke(&["identity", "show", "--", selector])
                .map_err(|error| {
                    // Office's existing selector reports invalid names as absent;
                    // the public core show command deliberately validates syntax.
                    if error.code == "INVALID_NAME" {
                        tmt_command_output::identity_missing(selector)
                    } else {
                        error
                    }
                })?;
            identity(&value, true)
        } else {
            let value = self.invoke(&["whoami"]).map_err(|error| {
                if error.code == "PANE_NOT_FOUND" {
                    Failure::new("IDENTITY_REQUIRED", "An identity is required; use --identity or run from a verified bound pane.", 1)
                } else {
                    error
                }
            })?;
            if value.get("bound").and_then(serde_json::Value::as_bool) != Some(true) {
                return Err(Failure::new(
                    "IDENTITY_REQUIRED",
                    "An identity is required; use --identity or run from a verified bound pane.",
                    1,
                ));
            }
            identity(&value, false)
        }
    }

    fn room_history(&self, selector: &str) -> Result<RoomHistory, Failure> {
        let value = self.invoke(&["room", "show", "--", selector])?;
        let id = value
            .get("room")
            .and_then(|room| room.get("id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| unavailable("Core room lookup omitted its UUID."))?;
        Ok(RoomHistory { id: id.into() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_error_preserves_public_code_status_and_correction_hint() {
        let json = br#"{"error":{"code":"CALLER_IDENTITY_AMBIGUOUS","message":"Ambiguous host.","suggestion":"Use --identity."}}"#;
        let failure = child_failure(json, 1).expect("bounded public error");
        assert_eq!(failure.status, 1);
        assert_eq!(failure.code, "CALLER_IDENTITY_AMBIGUOUS");
        assert_eq!(failure.message, "Ambiguous host.");
        assert_eq!(failure.document()["error"]["suggestion"], "Use --identity.");
        assert!(child_failure(br#"{"error":{"code":"bad","message":"x"}}"#, 1).is_none());
        assert!(child_failure(json, 0).is_none());
    }
}
