//! Remote's user settings: one small JSON file in the Remote state directory, no config system.
//! `{"open": true|false}` controls whether `pair` opens the browser; an absent
//! file or key means the default (on). Optional sessionsPerDevice limits sessions at open;
//! absent or explicit null means unlimited. Unknown keys are ignored so a newer file never breaks an
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
    default_remote: Option<Value>,
    remote_name: Option<String>,
    sessions_per_device: Option<usize>,
    sessions_configured: bool,
    /// The file exists but could not be read; defaults apply.
    pub malformed: bool,
}
impl RemoteSettings {
    pub fn remote_name(&self) -> Option<&str> {
        self.remote_name.as_deref()
    }
    pub fn default_remote(&self) -> Option<&str> {
        self.default_remote.as_ref().and_then(Value::as_str)
    }
    pub fn default_valid(&self) -> bool {
        self.default_remote.as_ref().is_none_or(|value| {
            value
                .as_str()
                .is_some_and(|s| s == "local" || crate::remote_catalog::name_ok(s))
        })
    }
    pub fn default_source(&self) -> &'static str {
        if self.default_remote.is_some() {
            FILE
        } else {
            "default"
        }
    }
    pub fn open(&self) -> bool {
        self.open.unwrap_or(true)
    }
    /// Where the effective `open` value comes from.
    pub fn source(&self) -> &'static str {
        if self.open.is_some() { FILE } else { "default" }
    }
    pub fn sessions_per_device(&self) -> Option<usize> {
        if self.sessions_configured {
            self.sessions_per_device
        } else {
            crate::limits::DEFAULT_SESSIONS_PER_DEVICE
        }
    }
    pub fn sessions_source(&self) -> &'static str {
        if self.sessions_configured {
            FILE
        } else {
            "default"
        }
    }
    pub fn json(&self) -> Value {
        json!({"open": self.open(), "source": self.source(), "sessionsPerDevice": self.sessions_per_device(), "sessionsPerDeviceSource": self.sessions_source()})
    }
    fn parse(bytes: &[u8]) -> Self {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) if value.is_object() => Self {
                open: value["open"].as_bool(),
                default_remote: value.get("defaultRemote").cloned(),
                remote_name: value
                    .get("remoteName")
                    .and_then(Value::as_str)
                    .filter(|name| crate::canonical::device_name(name))
                    .map(str::to_owned),
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
                default_remote: None,
                remote_name: None,
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
    read(root)
        .ok()
        .filter(|settings| !settings.malformed)
        .unwrap_or(RemoteSettings {
            open: None,
            default_remote: None,
            remote_name: None,
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
/// Only syntax is admitted here; the CLI validates the current catalog/owned project.
pub fn set_default_remote(root: &Path, reference: &str) -> Result<RemoteSettings> {
    if reference != "local" && !crate::remote_catalog::name_ok(reference) {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_INVALID",
            "Invalid remote name or project id.",
        )
        .with_hint("Run tmt remote settings default-remote local."));
    }
    set(root, "defaultRemote", json!(reference))
}
/// Set the local Remote name without rewriting an unchanged value.
pub fn set_remote_name(root: &Path, name: &str) -> Result<RemoteSettings> {
    set_remote_name_observed(root, name, |_| Ok(()))
}

fn set_remote_name_observed(
    root: &Path,
    name: &str,
    mut observe: impl FnMut(WriteStage) -> Result<()>,
) -> Result<RemoteSettings> {
    if !crate::canonical::device_name(name) {
        return Err(RemoteError::new(
            "REMOTE_INPUT_INVALID",
            "Remote name must be 1–64 bytes, nonblank, and contain no control characters.",
        ));
    }
    let mut touched = false;
    write_setting(root, "remoteName", json!(name), true, |stage| {
        if stage == WriteStage::BeforeTouch {
            touched = true;
        }
        observe(stage)
    }).map_err(|_| {
        if touched {
            RemoteError::new("REMOTE_SETTINGS_UNCONFIRMED", "The Remote name change may have started; read settings before deciding what to do next.")
        } else {
            RemoteError::new("REMOTE_SETTINGS_UNAVAILABLE", "The Remote name was not changed because settings could not be opened.")
        }
    })
}

/// Preserve other settings under the same read/write lock.
fn set(root: &Path, key: &str, value: Value) -> Result<RemoteSettings> {
    set_observed(root, key, value, |_| Ok(()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteStage {
    BeforeTouch,
    Truncated,
    Written,
    Synced,
}

/// The callback marks the uncertainty boundary before creation or truncation.
/// Lock ordering is live Session -> Store transaction -> settings.lock; the callback
/// must not acquire Session or Store. CLI setters use this same writer.
pub(crate) fn set_observed(
    root: &Path,
    key: &str,
    value: Value,
    observe: impl FnMut(WriteStage) -> Result<()>,
) -> Result<RemoteSettings> {
    write_setting(root, key, value, false, observe)
}

fn write_setting(
    root: &Path,
    key: &str,
    value: Value,
    skip_same: bool,
    mut observe: impl FnMut(WriteStage) -> Result<()>,
) -> Result<RemoteSettings> {
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
    if skip_same && document.get(key) == Some(&value) {
        return Ok(RemoteSettings::parse(document.to_string().as_bytes()));
    }
    document[key] = value;
    let bytes = document.to_string();
    if bytes.len() as u64 > LIMIT {
        return Err(RemoteError::new(
            "REMOTE_INPUT_INVALID",
            "Settings file is too large.",
        ));
    }
    observe(WriteStage::BeforeTouch)?;
    let mut file = layout.file(FILE)?;
    file.set_len(0)?;
    observe(WriteStage::Truncated)?;
    file.write_all(bytes.as_bytes())?;
    observe(WriteStage::Written)?;
    file.sync_all()?;
    observe(WriteStage::Synced)?;
    Ok(RemoteSettings::parse(bytes.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::RemoteSettings;
    #[test]
    fn absent_or_unreadable_session_setting_is_unlimited() {
        let root = std::env::temp_dir().join(format!("t4018-{}", crate::store::uuid_v4().unwrap()));
        assert_eq!(super::read_or_default(&root).sessions_per_device(), None);
        super::set_open(&root, true).unwrap();
        assert_eq!(super::read_or_default(&root).sessions_per_device(), None);
        std::fs::write(root.join("remote/settings.json"), "{").unwrap();
        let loaded = super::read_or_default(&root);
        assert!(loaded.malformed);
        assert_eq!(loaded.sessions_per_device(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

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

#[cfg(test)]
mod write_tests {
    use super::*;
    #[test]
    fn interruption_after_truncation_is_observable_as_malformed_defaults() {
        let root =
            std::env::temp_dir().join(format!("t1769-write-{}", crate::store::uuid_v4().unwrap()));
        set_open(&root, false).unwrap();
        let mut touched = false;
        let error = set_observed(&root, "open", json!(true), |stage| {
            if stage == WriteStage::BeforeTouch {
                touched = true;
            }
            if stage == WriteStage::Truncated {
                return Err(RemoteError::new("REMOTE_IO", "Injected after truncation."));
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "REMOTE_IO");
        assert!(touched);
        assert!(
            std::fs::read(root.join("remote/settings.json"))
                .unwrap()
                .is_empty()
        );
        let loaded = read_or_default(&root);
        assert!(loaded.malformed);
        assert!(loaded.open());
        assert_eq!(loaded.source(), "default");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn remote_name_same_value_does_not_touch_and_preserves_other_settings() {
        let root =
            std::env::temp_dir().join(format!("t5280-name-{}", crate::store::uuid_v4().unwrap()));
        set_open(&root, false).unwrap();
        set_remote_name(&root, "Studio Remote").unwrap();
        let file = root.join("remote/settings.json");
        let before = std::fs::read(&file).unwrap();
        let mut stages = Vec::new();
        let loaded = set_remote_name_observed(&root, "Studio Remote", |stage| {
            stages.push(stage);
            Ok(())
        })
        .unwrap();
        assert!(stages.is_empty(), "same value must never enter the writer");
        assert_eq!(std::fs::read(&file).unwrap(), before);
        assert_eq!(loaded.remote_name(), Some("Studio Remote"));
        assert!(!loaded.open());
        let invalid = set_remote_name(&root, "   ").unwrap_err();
        assert_eq!(invalid.code, "REMOTE_INPUT_INVALID");
        assert_eq!(std::fs::read(&file).unwrap(), before);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn remote_name_refusal_and_uncertainty_follow_first_touch() {
        let root =
            std::env::temp_dir().join(format!("t5280-fault-{}", crate::store::uuid_v4().unwrap()));
        std::fs::write(&root, b"not a directory").unwrap();
        let before_touch = set_remote_name(&root, "Studio").unwrap_err();
        assert_eq!(before_touch.code, "REMOTE_SETTINGS_UNAVAILABLE");
        assert_eq!(std::fs::read(&root).unwrap(), b"not a directory");
        std::fs::remove_file(&root).unwrap();
        set_remote_name(&root, "Old").unwrap();
        let after_touch = set_remote_name_observed(&root, "New", |stage| {
            if stage == WriteStage::Truncated {
                return Err(RemoteError::new("REMOTE_IO", "Injected after truncation."));
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(after_touch.code, "REMOTE_SETTINGS_UNCONFIRMED");
        assert!(
            std::fs::read(root.join("remote/settings.json"))
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn remote_name_interrupted_writer_is_not_retried_or_journaled() {
        use std::io::{BufRead, Read, Write};
        use std::os::unix::process::ExitStatusExt;
        use std::process::{Child, Command, Stdio};
        use std::sync::mpsc;
        use std::time::Duration;
        const TEST: &str =
            "settings::write_tests::remote_name_interrupted_writer_is_not_retried_or_journaled";
        const PREFIX: &str = "TMT_5280_STAGE ";
        if let Ok(root) = std::env::var("TMT_5280_ROOT") {
            let phase = std::env::var("TMT_5280_PHASE").unwrap();
            let result = set_remote_name_observed(Path::new(&root), "New Remote", |stage| {
                if (phase == "truncated" && stage == WriteStage::Truncated)
                    || (phase == "synced" && stage == WriteStage::Synced)
                {
                    println!("{PREFIX}{phase}");
                    std::io::stdout().flush().unwrap();
                    let mut release = [0];
                    std::io::stdin().read_exact(&mut release).unwrap();
                    panic!("parent must kill the paused writer");
                }
                Ok(())
            });
            panic!("writer returned without parent interruption: {result:?}");
        }
        struct OwnedChild(Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                if self.0.try_wait().ok().flatten().is_none() {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
        }
        for phase in ["truncated", "synced"] {
            let root = std::env::temp_dir()
                .join(format!("t5280-crash-{}", crate::store::uuid_v4().unwrap()));
            set_remote_name(&root, "Old Remote").unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env("TMT_5280_ROOT", &root)
                .env("TMT_5280_PHASE", phase)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut child = OwnedChild(child);
            let held_input = child.0.stdin.take().unwrap();
            let stdout = child.0.stdout.take().unwrap();
            let (send, receive) = mpsc::channel();
            let reader = std::thread::spawn(move || {
                for line in std::io::BufReader::new(stdout).lines() {
                    let line = line.unwrap();
                    if let Some(found) = line.strip_prefix(PREFIX) {
                        let _ = send.send(found.to_owned());
                        break;
                    }
                }
            });
            assert_eq!(
                receive.recv_timeout(Duration::from_secs(30)).unwrap(),
                phase
            );
            child.0.kill().unwrap();
            assert_eq!(child.0.wait().unwrap().signal(), Some(9));
            drop(held_input);
            reader.join().unwrap();
            let bytes = std::fs::read(root.join("remote/settings.json")).unwrap();
            if phase == "truncated" {
                assert!(bytes.is_empty());
                assert!(read_or_default(&root).malformed);
            } else {
                assert_eq!(read(&root).unwrap().remote_name(), Some("New Remote"));
            }
            assert!(
                !root.join("remote/remote.db").exists(),
                "local settings creates no receipt or journal"
            );
            assert_eq!(
                std::fs::read(root.join("remote/settings.json")).unwrap(),
                bytes,
                "recovery read must not resend"
            );
            std::fs::remove_dir_all(&root).unwrap();
        }
    }
}
