//! Disposable display values, never a View or a source of action authority.
//! The next first-frame consumer must treat every loaded value as stale.

use super::{app::Snapshot, home, tabs};
use crate::{cache, migration};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

const VERSION: u64 = 1;
const LIMIT: usize = 1024 * 1024;

/// Full squad inventory from the existing acquisition, including hidden squads.
pub(super) type Rooms = BTreeMap<String, String>;

/// Inert display JSON with a closed schema. It cannot become an App/View/Request.
pub(crate) struct Display(pub(super) Value);

pub(super) struct Store {
    root: PathBuf,
}

fn filename(key: &str) -> Option<String> {
    match key {
        tabs::ALL => Some("home.json".into()),
        tabs::LEADS => Some("leads.json".into()),
        _ => match tabs::user_name(key) {
            Some(name) if crate::squad::valid_name(name) && !tabs::reserved(name) => {
                Some(format!("tab-{name}.json"))
            }
            None if crate::squad::valid_name(key) => Some(format!("squad-{key}.json")),
            _ => None,
        },
    }
}

fn string(value: &Value) -> bool {
    value.as_str().is_some_and(|text| text.len() <= 4096)
}
fn optional_string(value: &Value) -> bool {
    value.is_null() || string(value)
}
fn object(value: &Value, keys: &[&str]) -> bool {
    value.as_object().is_some_and(|fields| {
        fields.len() == keys.len() && keys.iter().all(|key| fields.contains_key(*key))
    })
}
fn valid_array(value: &Value, valid: impl Fn(&Value) -> bool) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().all(valid))
}
fn key(value: &Value) -> bool {
    value.as_str().is_some_and(|key| filename(key).is_some())
}
fn row(value: &Value) -> Value {
    // Only named fields that the base member headings paint. In particular,
    // pending/request text becomes a decision mark, never copied content.
    json!({
        "name": value["name"].as_str(), "squad": value["squad"].as_str(),
        "state": value["state"].as_str(),
        "task": value["fields"]["task"].as_str(),
        "waiting": crate::attention::waits_on_you(value),
    })
}
fn valid_row(value: &Value) -> bool {
    object(value, &["name", "squad", "state", "task", "waiting"])
        && string(&value["name"])
        && ["squad", "state", "task"]
            .iter()
            .all(|field| optional_string(&value[*field]))
        && value["waiting"].is_boolean()
}
fn counts(value: &home::Counts) -> Value {
    json!({"members": value.members, "waiting": value.waiting, "blocked": value.blocked,
        "review": value.review, "working": value.working, "idle": value.idle})
}
fn valid_counts(value: &Value) -> bool {
    let fields = ["members", "waiting", "blocked", "review", "working", "idle"];
    object(value, &fields) && fields.iter().all(|field| value[*field].as_u64().is_some())
}
fn home(value: &home::Home) -> Value {
    json!({
        "summary": counts(&value.summary),
        "squads": value.squads.iter().map(|line| json!({
            "squad": line.squad, "lead": line.lead.as_ref().and_then(|lead| lead["name"].as_str()),
            "counts": counts(&line.counts), "members": counts(&line.members),
        })).collect::<Vec<_>>(),
        "sections": value.sections.iter().map(|section| json!({
            "key": section.key,
            "rows": section.rows.iter().map(|line| json!({
                "name": line.member["name"].as_str(), "squad": line.squad,
                "age": line.age.as_ref().map(|age| json!({
                    "source": match age.source { home::AgeSource::Request => "request", home::AgeSource::Observed => "observed" },
                    "sinceMs": age.since_ms,
                })),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}
fn valid_home(value: &Value) -> bool {
    object(value, &["summary", "squads", "sections"])
        && valid_counts(&value["summary"])
        && valid_array(&value["squads"], |line| {
            object(line, &["squad", "lead", "counts", "members"])
                && string(&line["squad"])
                && optional_string(&line["lead"])
                && valid_counts(&line["counts"])
                && valid_counts(&line["members"])
        })
        && valid_array(&value["sections"], |section| {
            object(section, &["key", "rows"])
                && matches!(section["key"].as_str(), Some("needs-you" | "blocked"))
                && valid_array(&section["rows"], |line| {
                    object(line, &["name", "squad", "age"])
                        && string(&line["name"])
                        && string(&line["squad"])
                        && (line["age"].is_null()
                            || (object(&line["age"], &["source", "sinceMs"])
                                && matches!(
                                    line["age"]["source"].as_str(),
                                    Some("request" | "observed")
                                )
                                && line["age"]["sinceMs"].as_u64().is_some()))
                })
        })
}

impl Display {
    /// The tab this value was projected for; `valid` checked it on load.
    pub(super) fn tab(&self) -> &str {
        self.0["tabKey"].as_str().unwrap_or_default()
    }
    pub(super) fn written_at_ms(&self) -> u64 {
        self.0["writtenAtMs"].as_u64().unwrap_or_default()
    }

    pub(super) fn project(snapshot: &Snapshot, rooms: &Rooms, written_at_ms: u64) -> Option<Self> {
        let view = snapshot.view.as_ref().ok()?;
        if view.document["partial"] == true
            || view
                .home
                .as_ref()
                .is_some_and(|home| home.incomplete || !home.failures.is_empty())
        {
            return None;
        }
        let tab = snapshot.squad.as_deref()?;
        let sections = view.document["sections"].as_array()?;
        let value = json!({
            "version": VERSION, "writtenAtMs": written_at_ms, "tabKey": tab,
            "rooms": rooms, "tabs": snapshot.tabs, "hidden": snapshot.hidden, "pinned": snapshot.pinned,
            "attention": snapshot.attention.iter().map(|(key, attention)| {
                (key.clone(), json!({"waiting": attention.waiting, "blocked": attention.blocked}))
            }).collect::<BTreeMap<_, _>>(),
            "view": {
                "lead": view.document["squad"]["lead"].is_object().then(|| row(&view.document["squad"]["lead"])),
                "sections": sections.iter().filter(|_| view.home.is_none()).map(|section| json!({
                    "title": section["title"].as_str(),
                    "rows": section["rows"].as_array().into_iter().flatten().map(row).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "home": view.home.as_ref().map(home),
            },
        });
        valid(&value, tab, rooms).then_some(Self(value))
    }
}

fn valid(value: &Value, tab: &str, rooms: &Rooms) -> bool {
    object(
        value,
        &[
            "version",
            "writtenAtMs",
            "tabKey",
            "rooms",
            "tabs",
            "hidden",
            "pinned",
            "attention",
            "view",
        ],
    ) && value["version"] == VERSION
        && value["writtenAtMs"].as_u64().is_some()
        && value["tabKey"] == tab
        && filename(tab).is_some()
        && value["rooms"] == json!(rooms)
        && rooms
            .iter()
            .all(|(name, id)| crate::squad::valid_name(name) && crate::config::uuid_like(id))
        && (tabs::aggregate(tab) || rooms.contains_key(tab))
        && valid_array(&value["tabs"], key)
        && valid_array(&value["hidden"], key)
        && value["pinned"]
            .as_u64()
            .zip(value["tabs"].as_array())
            .is_some_and(|(pinned, tabs)| pinned <= tabs.len() as u64)
        && value["attention"].as_object().is_some_and(|attention| {
            attention.iter().all(|(key, counts)| {
                filename(key).is_some()
                    && object(counts, &["waiting", "blocked"])
                    && counts["waiting"].as_u64().is_some()
                    && counts["blocked"].as_u64().is_some()
            })
        })
        && object(&value["view"], &["lead", "sections", "home"])
        && (value["view"]["lead"].is_null() || valid_row(&value["view"]["lead"]))
        && valid_array(&value["view"]["sections"], |section| {
            object(section, &["title", "rows"])
                && optional_string(&section["title"])
                && valid_array(&section["rows"], valid_row)
        })
        && (value["view"]["home"].is_null()
            || (tab == tabs::ALL && valid_home(&value["view"]["home"])))
}

/// Serialize within a fixed byte budget; an oversized candidate never reaches disk.
struct Bytes(Vec<u8>);
impl Write for Bytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > LIMIT.saturating_sub(self.0.len()) {
            return Err(io::Error::other("snapshot exceeds byte bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Store {
    pub(super) fn new(root: &Path) -> Option<Self> {
        root.is_absolute().then_some(())?;
        Some(Self {
            root: fs::canonicalize(root).ok()?,
        })
    }

    fn directory(&self, create: bool) -> io::Result<PathBuf> {
        let mut path = self.root.clone();
        migration::directory(&path)?;
        path.push("ops");
        migration::directory(&path)?;
        for component in ["cache", "board"] {
            path.push(component);
            if create && !path.try_exists()? {
                match fs::DirBuilder::new().mode(0o700).create(&path) {
                    Ok(()) => (),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                    Err(error) => return Err(error),
                }
            }
            migration::directory(&path)?;
            if fs::symlink_metadata(&path)?.permissions().mode() & 0o777 != 0o700 {
                return Err(io::Error::other("snapshot directory must be private"));
            }
        }
        Ok(path)
    }

    pub(super) fn write(&self, display: &Display, current: impl Fn() -> bool) -> io::Result<()> {
        if !current() || !self.root.join("ops").try_exists()? {
            return Ok(());
        }
        let mut bytes = Bytes(Vec::new());
        serde_json::to_writer(&mut bytes, &display.0).map_err(io::Error::other)?;
        let name = filename(
            display.0["tabKey"]
                .as_str()
                .ok_or(io::ErrorKind::InvalidInput)?,
        )
        .ok_or(io::ErrorKind::InvalidInput)?;
        let directory = self.directory(true)?;
        let path = directory.join(name);
        // Replacing corrupt JSON is expected; replacing an unsafe path is not.
        self.existing(&path)?;
        cache::replace_if(&path, &bytes.0, || {
            if !current() {
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.directory(false)?;
            self.existing(&path)?;
            Ok(())
        })
    }

    fn existing(&self, path: &Path) -> io::Result<()> {
        match migration::open(path) {
            Ok(file) if file.metadata()?.permissions().mode() & 0o777 == 0o600 => Ok(()),
            Ok(_) => Err(io::Error::other("snapshot file must be private")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// A read-only, always-stale value for the first frame. No action owner reads it.
    pub(super) fn load(&self, tab: &str, rooms: &Rooms) -> Option<Display> {
        let path = self.directory(false).ok()?.join(filename(tab)?);
        let file: File = migration::open(&path).ok()?;
        if file.metadata().ok()?.permissions().mode() & 0o777 != 0o600 {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(LIMIT as u64 + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() > LIMIT {
            return None;
        }
        let value = serde_json::from_slice(&bytes).ok()?;
        valid(&value, tab, rooms).then_some(Display(value))
    }
}

#[cfg(test)]
mod tests;
