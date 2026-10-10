//! Public Core process boundary; Digest never reads Core storage or configuration files.

use serde_json::Value;
use std::{
    ffi::OsString,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct Error {
    pub code: String,
    pub message: String,
}
impl Error {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::new("DIGEST_SETTINGS_IO", error.to_string())
    }
}

pub struct Core {
    program: PathBuf,
    deadline: Option<Instant>,
}
impl Core {
    #[cfg(test)]
    pub fn fixture(program: PathBuf) -> Self {
        Self {
            program,
            deadline: None,
        }
    }
    pub fn discover() -> Result<Self, Error> {
        let path = tmt_invoke::invoking_tmt()
            .map_err(|error| Error::new("CORE_UNAVAILABLE", error.to_string()))?;
        use std::os::unix::fs::MetadataExt;
        let current = std::env::current_exe()
            .map_err(|error| Error::new("CORE_UNAVAILABLE", error.to_string()))?;
        let same_path = std::fs::canonicalize(&path)
            .ok()
            .zip(std::fs::canonicalize(&current).ok())
            .is_some_and(|(a, b)| a == b);
        let same_file = std::fs::metadata(&path)
            .ok()
            .zip(std::fs::metadata(&current).ok())
            .is_some_and(|(a, b)| a.dev() == b.dev() && a.ino() == b.ino());
        if same_path || same_file {
            return Err(Error::new(
                "CORE_UNAVAILABLE",
                "TMT_EXECUTABLE selects Digest itself",
            ));
        }
        Ok(Self {
            program: path,
            deadline: None,
        })
    }
    pub fn until(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }
    pub fn json(&self, words: &[&str]) -> Result<Value, Error> {
        let args: Vec<OsString> = words
            .iter()
            .copied()
            .chain(["--json"])
            .map(Into::into)
            .collect();
        self.capture(&args, &[])
    }
    pub fn api(&self, operation: &str, input: Value) -> Result<Value, Error> {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 1, "operation": operation, "input": input,
        }))
        .map_err(|error| Error::new("DIGEST_INPUT_INVALID", error.to_string()))?;
        self.capture(&[OsString::from("api")], &bytes)
    }
    fn capture(&self, args: &[OsString], input: &[u8]) -> Result<Value, Error> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let output = tmt_invoke::invoke(
            tmt_invoke::Request {
                program: &self.program,
                args,
                input,
                deadline: self.deadline.map_or(deadline, |end| end.min(deadline)),
                max_stream_bytes: 1024 * 1024,
                launch: Default::default(),
            },
            None,
        )
        .map_err(|error| Error::new("CORE_UNAVAILABLE", error.to_string()))?;
        let document: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
            Error::new(
                "CORE_RESPONSE_INVALID",
                "Core did not return a JSON document",
            )
        })?;
        if !output.status.success() {
            return Err(Error::new(
                document["error"]["code"].as_str().unwrap_or("CORE_FAILED"),
                document["error"]["message"]
                    .as_str()
                    .unwrap_or("Core command failed"),
            ));
        }
        Ok(document)
    }
    pub fn member(&self, member: &str) -> Result<(String, String), Error> {
        let document = self.json(&["identity", "ls"])?;
        let identities = document["identities"]
            .as_array()
            .ok_or_else(|| Error::new("CORE_RESPONSE_INVALID", "Core returned no identities"))?;
        let identity = identities
            .iter()
            .find(|row| row["id"] == member || row["name"] == member)
            .ok_or_else(|| {
                Error::new(
                    "NAME_NOT_FOUND",
                    format!("Identity '{member}' was not found"),
                )
            })?;
        let id = identity["id"]
            .as_str()
            .filter(|id| crate::settings::canonical_uuid(id))
            .ok_or_else(|| {
                Error::new(
                    "CORE_RESPONSE_INVALID",
                    "Core returned no canonical identity UUID",
                )
            })?;
        let name = identity["name"]
            .as_str()
            .ok_or_else(|| Error::new("CORE_RESPONSE_INVALID", "Core returned no identity name"))?;
        Ok((id.into(), name.into()))
    }
    pub fn setter(&self) -> Result<Option<String>, Error> {
        match self.json(&["identity", "show"]) {
            Ok(document) => document["identity"]["id"]
                .as_str()
                .filter(|id| crate::settings::canonical_uuid(id))
                .map(|id| Some(id.into()))
                .ok_or_else(|| {
                    Error::new(
                        "CORE_RESPONSE_INVALID",
                        "Core returned no canonical setter UUID",
                    )
                }),
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "IDENTITY_REQUIRED" | "CALLER_IDENTITY_AMBIGUOUS"
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    pub fn settings_path(&self) -> Result<PathBuf, Error> {
        let shown = self.json(&["config", "show"])?;
        shown["paths"]["global"]
            .as_str()
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .and_then(|p| p.parent().map(|p| p.join("digest.toml")))
            .ok_or_else(|| {
                Error::new(
                    "CORE_RESPONSE_INVALID",
                    "Core reported no absolute global configuration path",
                )
            })
    }
}
