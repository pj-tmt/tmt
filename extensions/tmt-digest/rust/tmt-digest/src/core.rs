//! Public Core process boundary; Digest never reads Core storage or configuration files.

use serde_json::Value;
use std::{
    cell::RefCell,
    ffi::OsString,
    path::PathBuf,
    time::{Duration, Instant},
};
use tmt_invoke::{LineChannel, LineChannelOptions};

const FRAME_BYTES: usize = 1024 * 1024;

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
    /// Set by [`Core::persistent`]; every other caller keeps one process per request.
    persistent: Option<RefCell<Persistent>>,
}

/// One invocation's API transport, negotiated at its first request.
#[derive(Default)]
struct Persistent {
    /// The invoking CLI's `capabilities` document; absent until the first request.
    capabilities: Option<Value>,
    /// The one `tmt api --session` child, started by the first request that needs it. A failed
    /// exchange drops it and the next request starts a fresh one; no request is ever replayed.
    channel: Option<LineChannel>,
}
impl Core {
    #[cfg(test)]
    pub fn fixture(program: PathBuf) -> Self {
        Self {
            program,
            deadline: None,
            persistent: None,
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
            persistent: None,
        })
    }
    /// Send this handle's API requests through one `tmt api --session` child, when the invoking
    /// CLI advertises it, instead of one process per request. The child ends with the handle.
    pub fn persistent(mut self) -> Self {
        self.persistent = Some(RefCell::default());
        self
    }
    /// The `tmt` Digest was started by; ticks and reads go through it.
    pub fn program(&self) -> &std::path::Path {
        &self.program
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
        let Some(persistent) = &self.persistent else {
            return self.capture(&[OsString::from("api")], &bytes);
        };
        let mut state = persistent.borrow_mut();
        let capabilities = self.negotiate(&mut state)?;
        if operation == "capabilities" {
            return Ok(capabilities.clone());
        }
        if capabilities["session"]["framing"] != "ndjson" {
            drop(state);
            return self.capture(&[OsString::from("api")], &bytes);
        }
        self.exchange(&mut state, &bytes)
    }
    /// Whether this handle's requests go through a session, which is negotiated by the first
    /// request (or by this question). A caller uses it to ask only what a session makes cheap.
    pub fn in_session(&self) -> bool {
        let Some(persistent) = &self.persistent else {
            return false;
        };
        let mut state = persistent.borrow_mut();
        self.negotiate(&mut state)
            .is_ok_and(|capabilities| capabilities["session"]["framing"] == "ndjson")
    }
    /// Admission is one ordinary request; an older CLI answers without `session`.
    fn negotiate<'a>(&self, state: &'a mut Persistent) -> Result<&'a Value, Error> {
        if state.capabilities.is_none() {
            let request = br#"{"version":1,"operation":"capabilities","input":{}}"#;
            state.capabilities = Some(self.capture(&[OsString::from("api")], request)?);
        }
        Ok(state.capabilities.as_ref().expect("negotiated"))
    }
    /// The channel's answer is read like a one-shot process's: an error resource is a failure of
    /// this request only, except `API_SESSION_STALE`, after which Core has closed the session.
    fn exchange(&self, state: &mut Persistent, request: &[u8]) -> Result<Value, Error> {
        let deadline = self.call_deadline();
        let unavailable =
            |error: tmt_invoke::InvokeError| Error::new("CORE_UNAVAILABLE", error.to_string());
        if state.channel.is_none() {
            state.channel = Some(
                LineChannel::start(
                    LineChannelOptions {
                        program: &self.program,
                        args: &[OsString::from("api"), OsString::from("--session")],
                        deadline,
                        max_frame_bytes: FRAME_BYTES,
                        launch: Default::default(),
                    },
                    || false,
                )
                .map_err(unavailable)?,
            );
        }
        let reply = state
            .channel
            .as_mut()
            .expect("started")
            .exchange(request, deadline, || false);
        let reply = reply.map_err(|error| {
            // A failed exchange has already ended the child; whether Core acted is unknown.
            state.channel = None;
            unavailable(error)
        })?;
        let Ok(document) = serde_json::from_slice::<Value>(&reply) else {
            state.channel = None;
            return Err(Error::new(
                "CORE_RESPONSE_INVALID",
                "Core did not return a JSON document",
            ));
        };
        if document["error"].is_object() {
            let code = document["error"]["code"].as_str().unwrap_or("CORE_FAILED");
            if code == "API_SESSION_STALE" {
                state.channel = None;
            }
            return Err(Error::new(
                code,
                document["error"]["message"]
                    .as_str()
                    .unwrap_or("Core command failed"),
            ));
        }
        Ok(document)
    }
    fn call_deadline(&self) -> Instant {
        let deadline = Instant::now() + Duration::from_secs(15);
        self.deadline.map_or(deadline, |end| end.min(deadline))
    }
    fn capture(&self, args: &[OsString], input: &[u8]) -> Result<Value, Error> {
        let output = tmt_invoke::invoke(
            tmt_invoke::Request {
                program: &self.program,
                args,
                input,
                deadline: self.call_deadline(),
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
    pub fn data_root(&self) -> Result<PathBuf, Error> {
        self.api("storage.root", serde_json::json!({}))?["dataRoot"]
            .as_str()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                Error::new(
                    "CORE_RESPONSE_INVALID",
                    "Core reported no absolute data root",
                )
            })
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

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
