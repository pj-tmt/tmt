//! Remote's user settings: one small JSON file in the Remote state directory, no config system.
//! `{"open": true|false}` controls whether `pair` opens the browser; an absent
//! file or key means the default (on). Optional sessionsPerDevice limits sessions at open;
//! absent/null means unlimited. Unknown keys are ignored so a newer file never breaks an
//! older executable.
use crate::{error::RemoteError, state::Layout};
type Result<T> = std::result::Result<T, RemoteError>;
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

const FILE: &str = "settings.json";
const LOCK: &str = "settings.lock";
/// A settings file larger than this is not ours; it reads as malformed.
const LIMIT: u64 = 4096;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RemoteSettings {
    /// `None` when the file does not say: the default applies.
    open: Option<bool>,
    sessions_per_device: Option<usize>,
    sessions_configured: bool,
    /// The file exists but could not be read; defaults apply.
    pub malformed: bool,
}
impl RemoteSettings {
    pub fn open(&self) -> bool {
        self.open.unwrap_or(true)
    }
    /// Where the effective `open` value comes from.
    pub fn source(&self) -> &'static str {
        if self.open.is_some() { FILE } else { "default" }
    }
    pub fn sessions_per_device(&self) -> Option<usize> {
        self.sessions_per_device
    }
    pub fn sessions_source(&self) -> &'static str {
        if self.sessions_configured {
            FILE
        } else {
            "default"
        }
    }
    pub fn json(&self) -> Value {
        json!({"open": self.open(), "source": self.source(), "sessionsPerDevice": self.sessions_per_device, "sessionsPerDeviceSource": self.sessions_source()})
    }
    fn parse(bytes: &[u8]) -> Self {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) if value.is_object() => Self {
                open: value["open"].as_bool(),
                sessions_configured: value
                    .as_object()
                    .expect("object")
                    .contains_key("sessionsPerDevice"),
                sessions_per_device: value["sessionsPerDevice"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n > 0),
                malformed: (!value["open"].is_null() && !value["open"].is_boolean())
                    || (!value["sessionsPerDevice"].is_null()
                        && !value["sessionsPerDevice"]
                            .as_u64()
                            .is_some_and(|n| n > 0 && usize::try_from(n).is_ok())),
            },
            _ => Self {
                open: None,
                sessions_per_device: None,
                sessions_configured: false,
                malformed: true,
            },
        }
    }
}

/// What a person is told when the file is damaged or unreadable.
pub const UNREADABLE: &str = "settings.json could not be read; defaults apply";

/// How long a reader or setter waits for the lock before giving up on the file.
const LOCK_WAIT: Duration = Duration::from_secs(1);

/// The settings for a command whose effect already happened (a created page, a ready door):
/// any failure to read them is the defaults with `malformed` set, never an error.
pub fn read_or_default(root: &Path) -> RemoteSettings {
    read(root).unwrap_or(RemoteSettings {
        open: None,
        sessions_per_device: None,
        sessions_configured: false,
        malformed: true,
    })
}

/// The settings lock, waiting a bounded time for whoever holds it (a reader or a setter).
fn locked(layout: &Layout) -> Result<nix::fcntl::Flock<std::fs::File>> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match layout.lock(LOCK) {
            Ok(lock) => return Ok(lock),
            Err(tmt_extension_state::Error::Busy) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(crate::state::state_error(error)),
        }
    }
}

/// Read the settings of this data root. A missing directory or file is the defaults. The read
/// takes the setter's lock, so it never sees a half-written file.
pub fn read(root: &Path) -> Result<RemoteSettings> {
    let Some(layout) = Layout::existing(root)? else {
        return Ok(RemoteSettings::default());
    };
    let _lock = locked(&layout)?;
    match layout.read(FILE, LIMIT) {
        Ok(bytes) => Ok(RemoteSettings::parse(&bytes)),
        Err(tmt_extension_state::Error::ReadOpen(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(RemoteSettings::default())
        }
        Err(error) => Err(crate::state::state_error(error)),
    }
}

/// Set `open`. Two concurrent setters serialize on a lock; a crash mid-write leaves a malformed
/// file, which reads as the defaults with a warning rather than an error.
pub fn set_open(root: &Path, open: bool) -> Result<RemoteSettings> {
    set(root, "open", json!(open))
}
pub fn set_sessions_per_device(root: &Path, limit: Option<usize>) -> Result<RemoteSettings> {
    if limit == Some(0) {
        return Err(RemoteError::new(
            "REMOTE_INPUT_INVALID",
            "Session limit must be a positive integer or off.",
        ));
    }
    set(root, "sessionsPerDevice", json!(limit))
}
/// Preserve other settings under the same read/write lock.
fn set(root: &Path, key: &str, value: Value) -> Result<RemoteSettings> {
    let layout = Layout::open(root)?;
    let _lock = locked(&layout)?;
    let mut document = match layout.read(FILE, LIMIT) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({})),
        Err(tmt_extension_state::Error::ReadOpen(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            json!({})
        }
        Err(error) => return Err(crate::state::state_error(error)),
    };
    document[key] = value;
    let bytes = document.to_string();
    if bytes.len() as u64 > LIMIT {
        return Err(RemoteError::new(
            "REMOTE_INPUT_INVALID",
            "Settings file is too large.",
        ));
    }
    let mut file = layout.file(FILE)?;
    file.set_len(0)?;
    file.write_all(bytes.as_bytes())?;
    file.sync_all()?;
    Ok(RemoteSettings::parse(bytes.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::RemoteSettings;
    #[test]
    fn a_missing_key_is_the_default_and_unknown_keys_are_ignored() {
        let parsed = RemoteSettings::parse(br#"{"future":1}"#);
        assert!(parsed.open() && parsed.source() == "default" && !parsed.malformed);
        let off = RemoteSettings::parse(br#"{"open":false,"future":[1]}"#);
        assert!(!off.open() && off.source() == "settings.json" && !off.malformed);
    }
    #[test]
    fn anything_else_is_malformed_and_reads_as_the_default() {
        for bad in ["", "not json", "[]", r#"{"open":"yes"}"#, r#"{"open":1}"#] {
            let parsed = RemoteSettings::parse(bad.as_bytes());
            assert!(parsed.malformed && parsed.open(), "{bad}");
        }
    }
}
