//! Colab's user settings: one small JSON file in the Colab state directory, no config system.
//! `{"open": true|false}` controls whether `serve` and `page create` open the browser. An absent
//! file or key means the default. Unknown keys are ignored so a newer file never breaks an older
//! executable, and a setter keeps the keys it does not own (the task-era `wip` and `github` keys
//! are now simply unknown). `{"pageExpiry": N|"off"}` is the space's days without activity before
//! a page is archived (`page_expiry.rs`).
//! `{"defaultTemplate": NAME|ID}` is the template `page create` starts from when none is named
//! (`templates.rs`); absent means TMT's `starter`.
use crate::{
    Result,
    keyring::{Layout, StateFault},
    page_expiry::{MAX_DAYS, SpaceExpiry},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tmt_extension_state::{Document, StateError, UpdateError};

const FILE: &str = "settings.json";
/// A settings file larger than this is not ours; it reads as malformed.
const LIMIT: u64 = 4096;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ColabSettings {
    /// `None` when the file does not say: the default applies.
    open: Option<bool>,
    page_expiry: SpaceExpiry,
    /// The space's default template, a name or a page ID; `None` is TMT's `starter`.
    default_template: Option<String>,
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
    /// The space's days without activity before a page is archived.
    pub fn page_expiry(&self) -> SpaceExpiry {
        self.page_expiry
    }
    /// The template a `page create` without `--template` starts from; `None` is `starter`.
    pub fn default_template(&self) -> Option<&str> {
        self.default_template.as_deref()
    }
    pub fn json(&self) -> Value {
        let mut settings = json!({"open": self.open(), "source": self.source(),
            "pageExpiryDays": self.page_expiry.days()});
        // Only a space that chose a default template says so.
        if let Some(template) = &self.default_template {
            settings["defaultTemplate"] = json!(template);
        }
        settings
    }
    fn parse(bytes: &[u8]) -> Self {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) if value.is_object() => {
                let expiry = parse_expiry(&value["pageExpiry"]);
                let template = parse_default_template(&value["defaultTemplate"]);
                Self {
                    open: value["open"].as_bool(),
                    malformed: (!value["open"].is_null() && !value["open"].is_boolean())
                        || expiry.is_none()
                        || template.is_none(),
                    page_expiry: expiry.unwrap_or_default(),
                    default_template: template.flatten(),
                }
            }
            _ => Self {
                malformed: true,
                ..Self::default()
            },
        }
    }
}
/// `None` when the value is present but neither a day count nor `"off"`; absent is the default.
fn parse_expiry(value: &Value) -> Option<SpaceExpiry> {
    match value {
        Value::Null => Some(SpaceExpiry::Default),
        Value::String(text) if text == "off" => Some(SpaceExpiry::Off),
        value => {
            let days = value.as_u64()?;
            (1..=MAX_DAYS)
                .contains(&days)
                .then_some(SpaceExpiry::Days(days))
        }
    }
}

/// `None` when the value is present but not a template name or ID; absent is `starter`.
fn parse_default_template(value: &Value) -> Option<Option<String>> {
    match value {
        Value::Null => Some(None),
        Value::String(text) => valid_default_template(text).then(|| Some(text.clone())),
        _ => None,
    }
}

/// A template reference: a name (at most 40 characters) or a page ID (36), one line, no spaces.
pub fn valid_default_template(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 40
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// What a person is told when the file is damaged or unreadable.
pub const UNREADABLE: &str = "settings.json could not be read; defaults apply";

/// How long a scan waits for the work-index lock before giving up: the shared state wait bound.
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// The settings for a command whose effect already happened (a created page, a ready door):
/// any failure to read them is the defaults with `malformed` set, never an error.
pub fn read_or_default(root: &Path) -> ColabSettings {
    read(root).unwrap_or(ColabSettings {
        malformed: true,
        ..ColabSettings::default()
    })
}

/// A named state lock, waiting a bounded time for whoever holds it.
pub(crate) fn locked(layout: &Layout, name: &str) -> Result<nix::fcntl::Flock<std::fs::File>> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match layout.lock(name) {
            Ok(lock) => return Ok(lock),
            Err(tmt_extension_state::Error::Busy) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(crate::keyring::state_error(error)),
        }
    }
}

/// The settings document: the file is replaced whole by an atomic rename under the shared document
/// lock, so a reader takes no lock and never sees a half-written file. `retry` is the command that
/// repeats a change a refusal names.
fn document(layout: &Layout, retry: &str) -> Result<Document> {
    // The shared lock files live in the data root, beside the extension directory.
    let state_root = layout
        .directory
        .parent()
        .ok_or(StateFault::RootNotAbsolute)?;
    Document::new(state_root, &layout.directory.join(FILE), LIMIT, retry).map_err(document_error)
}

/// A refusal of the shared state entry as Colab words it: an unsafe file keeps its code, and every
/// other refusal (a wait that ran out, an oversized file) says which file and, where it can be
/// repeated, the exact retry.
fn document_error(error: StateError) -> Box<dyn std::error::Error + Send + Sync> {
    match error {
        StateError::Unsafe { .. } => StateFault::UnsafeFile.into(),
        error => error.into(),
    }
}

/// Read the settings of this data root. A missing directory or file is the defaults. A setter
/// replaces the file whole, so the read never sees a half-written file and takes no lock.
pub fn read(root: &Path) -> Result<ColabSettings> {
    let Some(layout) = Layout::existing(root)? else {
        return Ok(ColabSettings::default());
    };
    match document(&layout, "tmt colab settings")?
        .read()
        .map_err(document_error)?
    {
        Some(bytes) => Ok(ColabSettings::parse(&bytes)),
        None => Ok(ColabSettings::default()),
    }
}

/// Change the settings in one document update: `change` edits the file's own object, so a key this
/// setter does not own is kept. Two concurrent setters serialize and neither loses its change; a
/// crash leaves the old file whole (the update is a rename). A file that is not an object starts
/// again from an empty one; one past the byte bound is not ours and is refused, never replaced.
fn update(
    root: &Path,
    retry: &str,
    change: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> Result<ColabSettings> {
    let layout = Layout::open(root)?;
    let mut saved = Vec::new();
    document(&layout, retry)?
        .update(|current| {
            let mut table = match current.map(serde_json::from_slice::<Value>) {
                Some(Ok(Value::Object(table))) => table,
                _ => serde_json::Map::new(),
            };
            change(&mut table);
            saved = Value::Object(table).to_string().into_bytes();
            Ok::<_, std::convert::Infallible>(saved.clone())
        })
        .map_err(|error| match error {
            UpdateError::State(error) => document_error(error),
            UpdateError::Change(never) => match never {},
        })?;
    Ok(ColabSettings::parse(&saved))
}

/// Set `open`.
pub fn set_open(root: &Path, open: bool) -> Result<ColabSettings> {
    let retry = format!(
        "tmt colab settings open {}",
        if open { "on" } else { "off" }
    );
    update(root, &retry, |table| {
        table.insert("open".into(), json!(open));
    })
}

/// Set the space's days without activity before a page is archived; `Default` removes the key.
pub fn set_page_expiry(root: &Path, expiry: SpaceExpiry) -> Result<ColabSettings> {
    let retry = format!(
        "tmt colab settings page-expiry {}",
        match expiry {
            SpaceExpiry::Default => "default".to_owned(),
            SpaceExpiry::Days(days) => days.to_string(),
            SpaceExpiry::Off => "off".to_owned(),
        }
    );
    update(root, &retry, |table| {
        table.remove("pageExpiry");
        match expiry {
            SpaceExpiry::Default => {}
            SpaceExpiry::Days(days) => {
                table.insert("pageExpiry".into(), json!(days));
            }
            SpaceExpiry::Off => {
                table.insert("pageExpiry".into(), json!("off"));
            }
        }
    })
}

/// Set the space's default template (a name or a page ID), or return it to `starter` (`None`).
pub fn set_default_template(root: &Path, template: Option<&str>) -> Result<ColabSettings> {
    let retry = format!(
        "tmt colab template default {}",
        template.unwrap_or("--reset")
    );
    update(root, &retry, |table| {
        table.remove("defaultTemplate");
        if let Some(template) = template {
            table.insert("defaultTemplate".into(), json!(template));
        }
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
    fn page_expiry_reads_days_or_off_and_anything_else_is_malformed() {
        use crate::page_expiry::SpaceExpiry::{Days, Default, Off};
        for (text, expiry, days) in [
            ("{}", Default, Some(180)),
            (r#"{"pageExpiry":90}"#, Days(90), Some(90)),
            (r#"{"pageExpiry":3650}"#, Days(3650), Some(3650)),
            (r#"{"pageExpiry":"off"}"#, Off, None),
        ] {
            let parsed = ColabSettings::parse(text.as_bytes());
            assert!(!parsed.malformed, "{text}");
            assert_eq!(
                (parsed.page_expiry(), parsed.page_expiry().days()),
                (expiry, days)
            );
        }
        for bad in [
            r#"{"pageExpiry":0}"#,
            r#"{"pageExpiry":3651}"#,
            r#"{"pageExpiry":-1}"#,
            r#"{"pageExpiry":1.5}"#,
            r#"{"pageExpiry":"90"}"#,
            r#"{"pageExpiry":"forever"}"#,
            r#"{"pageExpiry":true}"#,
        ] {
            let parsed = ColabSettings::parse(bad.as_bytes());
            assert!(parsed.malformed && parsed.page_expiry() == Default, "{bad}");
        }
    }
    #[test]
    fn task_era_keys_are_unknown_and_never_make_the_file_malformed() {
        let parsed = ColabSettings::parse(br#"{"wip":{"default":3},"github":{"repo":"x"}}"#);
        assert!(!parsed.malformed);
        assert_eq!(parsed, ColabSettings::parse(b"{}"));
        assert!(parsed.json().get("wipLimit").is_none());
    }

    #[test]
    fn the_default_template_reads_a_name_or_id_and_a_bad_one_is_malformed() {
        let parsed = ColabSettings::parse(br#"{"defaultTemplate":"weekly-review"}"#);
        assert!(!parsed.malformed);
        assert_eq!(parsed.default_template(), Some("weekly-review"));
        assert_eq!(parsed.json()["defaultTemplate"], "weekly-review");
        let by_id =
            ColabSettings::parse(br#"{"defaultTemplate":"3f9b02c7-0000-4000-8000-000000000001"}"#);
        assert_eq!(
            by_id.default_template(),
            Some("3f9b02c7-0000-4000-8000-000000000001")
        );
        let absent = ColabSettings::parse(b"{}");
        assert_eq!(absent.default_template(), None);
        assert!(absent.json().get("defaultTemplate").is_none());
        for bad in [
            r#"{"defaultTemplate":""}"#,
            r#"{"defaultTemplate":3}"#,
            r#"{"defaultTemplate":"Weekly"}"#,
            r#"{"defaultTemplate":"two words"}"#,
            r#"{"defaultTemplate":"has/slash"}"#,
        ] {
            let parsed = ColabSettings::parse(bad.as_bytes());
            assert!(
                parsed.malformed && parsed.default_template().is_none(),
                "{bad}"
            );
        }
    }
    #[test]
    fn anything_else_is_malformed_and_reads_as_the_default() {
        for bad in ["", "not json", "[]", r#"{"open":"yes"}"#, r#"{"open":1}"#] {
            let parsed = ColabSettings::parse(bad.as_bytes());
            assert!(parsed.malformed && parsed.open(), "{bad}");
        }
    }
}
