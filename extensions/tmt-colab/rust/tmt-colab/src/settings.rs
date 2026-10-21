//! Colab's user settings: one small JSON file in the Colab state directory, no config system.
//! `{"open": true|false}` controls whether `serve` and `page create` open the browser;
//! `{"wip": {"default": N, "agents": {NAME: N}}}` is the work-in-progress limit (`wip.rs`). An
//! absent file or key means the default. Unknown keys are ignored so a newer file never breaks an
//! older executable, and a setter keeps the keys it does not own. `{"github": {"repo": "OWNER/REPO"}}`
//! is the repository the GitHub driver pushes tasks to (`work_github.rs`). `{"pageExpiry": N|"off"}`
//! is the space's days without activity before a page is archived (`page_expiry.rs`).
use crate::{
    Result,
    keyring::{Layout, StateFault},
    page_expiry::{MAX_DAYS, SpaceExpiry},
    wip::{Limits, MAX_LIMIT},
    work_github::Repo,
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
    wip: Limits,
    /// The repository the GitHub driver pushes tasks to; `None` until one is set.
    github_repo: Option<Repo>,
    page_expiry: SpaceExpiry,
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
    /// The work-in-progress limits; the defaults when the file does not say.
    pub fn wip(&self) -> &Limits {
        &self.wip
    }
    /// The repository the GitHub driver pushes tasks to.
    pub fn github_repo(&self) -> Option<&Repo> {
        self.github_repo.as_ref()
    }
    /// The space's days without activity before a page is archived.
    pub fn page_expiry(&self) -> SpaceExpiry {
        self.page_expiry
    }
    pub fn json(&self) -> Value {
        let mut settings = json!({"open": self.open(), "source": self.source(),
            "wipLimit": {"default": self.wip.default.unwrap_or(crate::wip::DEFAULT_LIMIT),
                "agents": self.wip.agents},
            "pageExpiryDays": self.page_expiry.days()});
        // Present only once a repository is set, so a space that never uses the driver reads as before.
        if let Some(repo) = &self.github_repo {
            settings["githubRepo"] = json!(repo.slug());
        }
        settings
    }
    fn parse(bytes: &[u8]) -> Self {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) if value.is_object() => {
                let wip = parse_wip(&value["wip"]);
                let github = parse_github(&value["github"]);
                let expiry = parse_expiry(&value["pageExpiry"]);
                Self {
                    open: value["open"].as_bool(),
                    malformed: (!value["open"].is_null() && !value["open"].is_boolean())
                        || (!value["wip"].is_null() && wip.is_none())
                        || github.is_none()
                        || expiry.is_none(),
                    wip: wip.unwrap_or_default(),
                    github_repo: github.flatten(),
                    page_expiry: expiry.unwrap_or_default(),
                }
            }
            _ => Self {
                malformed: true,
                ..Self::default()
            },
        }
    }
}
fn limit(value: &Value) -> Option<u32> {
    let n = u32::try_from(value.as_u64()?).ok()?;
    (1..=MAX_LIMIT).contains(&n).then_some(n)
}
/// `None` when the value is present but not a limit table; an absent value is the defaults.
fn parse_wip(value: &Value) -> Option<Limits> {
    if value.is_null() {
        return Some(Limits::default());
    }
    let table = value.as_object()?;
    let default = match table.get("default") {
        None => None,
        Some(value) => Some(limit(value)?),
    };
    let mut agents = std::collections::BTreeMap::new();
    for (name, value) in table
        .get("agents")
        .map_or(Some(&serde_json::Map::new()), |v| v.as_object())?
    {
        agents.insert(name.clone(), limit(value)?);
    }
    Some(Limits { default, agents })
}

/// `None` when the value is present but not `{"repo": "OWNER/REPO"}`; absent is no repository.
fn parse_github(value: &Value) -> Option<Option<Repo>> {
    if value.is_null() {
        return Some(None);
    }
    let repo = value.as_object()?.get("repo")?.as_str()?;
    Some(Some(Repo::parse(repo)?))
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

/// Set the space default limit (`agent` is `None`), a limit of one agent, or clear a limit
/// (`limit` is `None`): clearing the default returns it to `wip::DEFAULT_LIMIT`.
pub fn set_wip_limit(
    root: &Path,
    agent: Option<&str>,
    limit: Option<u32>,
) -> Result<ColabSettings> {
    let retry = match (agent, limit) {
        (None, Some(limit)) => format!("tmt colab settings wip-limit {limit}"),
        (None, None) => "tmt colab settings wip-limit".to_owned(),
        (Some(agent), Some(limit)) => format!("tmt colab settings wip-limit {agent} {limit}"),
        (Some(agent), None) => format!("tmt colab settings wip-limit {agent} default"),
    };
    update(root, &retry, |table| {
        let mut wip = match table.remove("wip") {
            Some(Value::Object(wip)) => wip,
            _ => serde_json::Map::new(),
        };
        match agent {
            None => {
                wip.remove("default");
                if let Some(limit) = limit {
                    wip.insert("default".into(), json!(limit));
                }
            }
            Some(agent) => {
                let mut agents = match wip.remove("agents") {
                    Some(Value::Object(agents)) => agents,
                    _ => serde_json::Map::new(),
                };
                agents.remove(agent);
                if let Some(limit) = limit {
                    agents.insert(agent.into(), json!(limit));
                }
                if !agents.is_empty() {
                    wip.insert("agents".into(), Value::Object(agents));
                }
            }
        }
        if !wip.is_empty() {
            table.insert("wip".into(), Value::Object(wip));
        }
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

/// Set the repository the GitHub driver pushes tasks to, or clear it (`repo` is `None`).
pub fn set_github_repo(root: &Path, repo: Option<&Repo>) -> Result<ColabSettings> {
    let retry = format!(
        "tmt colab settings github-repo {}",
        repo.map_or("none".to_owned(), |repo| repo.slug())
    );
    update(root, &retry, |table| {
        table.remove("github");
        if let Some(repo) = repo {
            table.insert("github".into(), json!({"repo": repo.slug()}));
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
    fn work_limits_read_and_a_bad_limit_is_malformed() {
        let parsed = ColabSettings::parse(br#"{"wip":{"default":3,"agents":{"ada":2}}}"#);
        assert!(!parsed.malformed);
        assert_eq!((parsed.wip().of("ada"), parsed.wip().of("bo")), (2, 3));
        assert_eq!(ColabSettings::parse(b"{}").wip().of("ada"), 1);
        for bad in [
            r#"{"wip":1}"#,
            r#"{"wip":{"default":0}}"#,
            r#"{"wip":{"default":1001}}"#,
            r#"{"wip":{"agents":{"ada":"2"}}}"#,
            r#"{"wip":{"agents":[]}}"#,
        ] {
            let parsed = ColabSettings::parse(bad.as_bytes());
            assert!(parsed.malformed && parsed.wip().of("ada") == 1, "{bad}");
        }
    }
    #[test]
    fn the_github_repo_reads_and_a_bad_one_is_malformed() {
        let parsed = ColabSettings::parse(br#"{"github":{"repo":"pj-tmt/tmt"}}"#);
        assert!(!parsed.malformed);
        assert_eq!(parsed.github_repo().unwrap().slug(), "pj-tmt/tmt");
        assert_eq!(parsed.json()["githubRepo"], "pj-tmt/tmt");
        assert!(ColabSettings::parse(b"{}").github_repo().is_none());
        for bad in [
            r#"{"github":1}"#,
            r#"{"github":{}}"#,
            r#"{"github":{"repo":3}}"#,
            r#"{"github":{"repo":"-x/y"}}"#,
            r#"{"github":{"repo":"x"}}"#,
        ] {
            let parsed = ColabSettings::parse(bad.as_bytes());
            assert!(parsed.malformed && parsed.github_repo().is_none(), "{bad}");
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
