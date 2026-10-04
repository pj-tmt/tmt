//! Colab's user settings: one small JSON file in the Colab state directory, no config system.
//! `{"open": true|false}` controls whether `serve` and `page create` open the browser; an absent
//! file or key means the default (on). Unknown keys are ignored so a newer file never breaks an
//! older executable.
use crate::{Result, keyring::Layout};
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
pub struct ColabSettings {
    /// `None` when the file does not say: the default applies.
    open: Option<bool>,
    /// The file exists but could not be read; defaults apply.
    pub malformed: bool,
}
impl ColabSettings {
    pub fn open(&self) -> bool {
        self.open.unwrap_or(true)
    }
    /// Where the effective `open` value comes from.
    pub fn source(&self) -> &'static str {
        if self.open.is_some() { FILE } else { "default" }
    }
    pub fn json(&self) -> Value {
        json!({"open": self.open(), "source": self.source()})
    }
    fn parse(bytes: &[u8]) -> Self {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) if value.is_object() => Self {
                open: value["open"].as_bool(),
                malformed: !value["open"].is_null() && !value["open"].is_boolean(),
            },
            _ => Self {
                open: None,
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
pub fn read_or_default(root: &Path) -> ColabSettings {
    read(root).unwrap_or(ColabSettings {
        open: None,
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
            Err(error) => return Err(crate::keyring::state_error(error)),
        }
    }
}

/// Read the settings of this data root. A missing directory or file is the defaults. The read
/// takes the setter's lock, so it never sees a half-written file.
pub fn read(root: &Path) -> Result<ColabSettings> {
    let Some(layout) = Layout::existing(root)? else {
        return Ok(ColabSettings::default());
    };
    let _lock = locked(&layout)?;
    match layout.read(FILE, LIMIT) {
        Ok(bytes) => Ok(ColabSettings::parse(&bytes)),
        Err(tmt_extension_state::Error::ReadOpen(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(ColabSettings::default())
        }
        Err(error) => Err(crate::keyring::state_error(error)),
    }
}

/// Set `open`. Two concurrent setters serialize on a lock; a crash mid-write leaves a malformed
/// file, which reads as the defaults with a warning rather than an error.
pub fn set_open(root: &Path, open: bool) -> Result<ColabSettings> {
    let layout = Layout::open(root)?;
    let _lock = locked(&layout)?;
    let mut file = layout.file(FILE)?;
    file.set_len(0)?;
    file.write_all(json!({"open": open}).to_string().as_bytes())?;
    file.sync_all()?;
    Ok(ColabSettings {
        open: Some(open),
        malformed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::ColabSettings;
    #[test]
    fn a_missing_key_is_the_default_and_unknown_keys_are_ignored() {
        let parsed = ColabSettings::parse(br#"{"future":1}"#);
        assert!(parsed.open() && parsed.source() == "default" && !parsed.malformed);
        let off = ColabSettings::parse(br#"{"open":false,"future":[1]}"#);
        assert!(!off.open() && off.source() == "settings.json" && !off.malformed);
    }
    #[test]
    fn anything_else_is_malformed_and_reads_as_the_default() {
        for bad in ["", "not json", "[]", r#"{"open":"yes"}"#, r#"{"open":1}"#] {
            let parsed = ColabSettings::parse(bad.as_bytes());
            assert!(parsed.malformed && parsed.open(), "{bad}");
        }
    }
}
