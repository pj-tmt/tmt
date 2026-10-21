//! Colab's user settings: one small JSON file in the Colab state directory, no config system.
//! `{"open": true|false}` controls whether `serve` and `page create` open the browser;
//! `{"wip": {"default": N, "agents": {NAME: N}}}` is the work-in-progress limit (`wip.rs`). An
//! absent file or key means the default. Unknown keys are ignored so a newer file never breaks an
//! older executable, and a setter keeps the keys it does not own. `{"github": {"repo": "OWNER/REPO"}}`
//! is the repository the GitHub driver pushes tasks to (`work_github.rs`).
use crate::{
    Result,
    keyring::Layout,
    wip::{Limits, MAX_LIMIT},
    work_github::Repo,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
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
    wip: Limits,
    /// The repository the GitHub driver pushes tasks to; `None` until one is set.
    github_repo: Option<Repo>,
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
    pub fn json(&self) -> Value {
        let mut settings = json!({"open": self.open(), "source": self.source(),
            "wipLimit": {"default": self.wip.default.unwrap_or(crate::wip::DEFAULT_LIMIT),
                "agents": self.wip.agents}});
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
                Self {
                    open: value["open"].as_bool(),
                    malformed: (!value["open"].is_null() && !value["open"].is_boolean())
                        || (!value["wip"].is_null() && wip.is_none())
                        || github.is_none(),
                    wip: wip.unwrap_or_default(),
                    github_repo: github.flatten(),
                }
            }
            _ => Self {
                open: None,
                wip: Limits::default(),
                github_repo: None,
                malformed: true,
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

/// What a person is told when the file is damaged or unreadable.
pub const UNREADABLE: &str = "settings.json could not be read; defaults apply";

/// How long a reader or setter waits for the lock before giving up on the file.
const LOCK_WAIT: Duration = Duration::from_secs(1);

/// The settings for a command whose effect already happened (a created page, a ready door):
/// any failure to read them is the defaults with `malformed` set, never an error.
pub fn read_or_default(root: &Path) -> ColabSettings {
    read(root).unwrap_or(ColabSettings {
        open: None,
        wip: Limits::default(),
        github_repo: None,
        malformed: true,
    })
}

/// A named state lock, waiting a bounded time for whoever holds it (a reader or a setter).
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

/// Read the settings of this data root. A missing directory or file is the defaults. The read
/// takes the setter's lock, so it never sees a half-written file.
pub fn read(root: &Path) -> Result<ColabSettings> {
    let Some(layout) = Layout::existing(root)? else {
        return Ok(ColabSettings::default());
    };
    let _lock = locked(&layout, LOCK)?;
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

/// Change the settings under the setter's lock: `change` edits the file's own object, so a key this
/// setter does not own is kept. Two concurrent setters serialize; a crash mid-write leaves a
/// malformed file, which reads as the defaults with a warning rather than an error. A file that is
/// not an object starts again from an empty one.
fn update(
    root: &Path,
    change: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> Result<ColabSettings> {
    let layout = Layout::open(root)?;
    let _lock = locked(&layout, LOCK)?;
    let mut file = layout.file(FILE)?;
    let mut text = Vec::new();
    std::io::Read::take(&mut file, LIMIT + 1).read_to_end(&mut text)?;
    let mut table = match serde_json::from_slice::<Value>(&text) {
        Ok(Value::Object(table)) if text.len() as u64 <= LIMIT => table,
        _ => serde_json::Map::new(),
    };
    change(&mut table);
    let bytes = Value::Object(table).to_string();
    file.set_len(0)?;
    std::io::Seek::rewind(&mut file)?;
    file.write_all(bytes.as_bytes())?;
    file.sync_all()?;
    Ok(ColabSettings::parse(bytes.as_bytes()))
}

/// Set `open`.
pub fn set_open(root: &Path, open: bool) -> Result<ColabSettings> {
    update(root, |table| {
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
    update(root, |table| {
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

/// Set the repository the GitHub driver pushes tasks to, or clear it (`repo` is `None`).
pub fn set_github_repo(root: &Path, repo: Option<&Repo>) -> Result<ColabSettings> {
    update(root, |table| {
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
