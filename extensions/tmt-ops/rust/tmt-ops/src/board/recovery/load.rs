//! Finite public-command jobs. No file, host, provider or retry owner here.
use super::model::*;
use crate::core::{Core, SquadError};
use serde_json::Value;
use std::time::Duration;

/// Which recovery store a job names. Only Core decides it: the first listing names none, and
/// Core's answer fixes the store for every later job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Store {
    Socket(String),
    /// The built-in host's members.
    Builtin,
}
impl Store {
    fn arguments(&self) -> [&str; 2] {
        match self {
            Self::Socket(socket) => ["--socket", socket],
            Self::Builtin => ["--host", "pty"],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub store: Store,
    pub name: Option<String>,
    pub digest: String,
    /// tmux only: windows and shells without agents or commands.
    pub layout_only: bool,
    /// Built-in only: each member's driver in a new conversation.
    pub fresh: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Job {
    List(Option<Store>),
    Show { store: Store, name: Option<String> },
    Save { store: Store, name: String },
    Remove { store: Store, name: String },
    Restore(Selection),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Task {
    pub owner: u64,
    pub serial: u64,
    pub job: Job,
}
#[derive(Debug)]
pub(super) enum Outcome {
    List(Inventory),
    Show(Box<Preview>),
    Members(MemberPreview),
    Saved(RecoverySnapshotSummary),
    Removed,
    Restored(Restore),
    MembersRestored(MemberRestore),
}
pub(crate) struct Completed {
    pub task: Task,
    pub(super) result: Result<Outcome, SquadError>,
}
impl Job {
    pub fn argv(&self) -> Vec<String> {
        let (command, store, name) = match self {
            Self::List(store) => ("ls", store.as_ref(), None),
            Self::Show { store, name } => ("show", Some(store), name.as_deref()),
            Self::Save { store, name } => ("save", Some(store), Some(name.as_str())),
            Self::Remove { store, name } => ("rm", Some(store), Some(name.as_str())),
            Self::Restore(s) => ("restore", Some(&s.store), s.name.as_deref()),
        };
        let mut argv = vec!["workspace".into(), command.into()];
        if let Some(store) = store {
            argv.extend(store.arguments().map(str::to_owned));
        }
        if let Self::Restore(s) = self {
            argv.extend(["--expect".into(), s.digest.clone()]);
            if s.layout_only {
                argv.push("--layout-only".into());
            }
            if s.fresh {
                argv.push("--fresh".into());
            }
        }
        if let Some(name) = name {
            argv.push(name.into());
        }
        argv
    }
    pub fn writes(&self) -> bool {
        matches!(
            self,
            Self::Save { .. } | Self::Remove { .. } | Self::Restore(_)
        )
    }
}
fn decode<T: Decode>(value: Value) -> Result<T, SquadError> {
    T::decode(&value)
}
pub(crate) fn complete(core: &Core, task: Task) -> Completed {
    let result = execute(core, &task.job);
    Completed { task, result }
}
/// Core starts built-in members one at a time within its own two-minute budget.
const BUILTIN_RESTORE: Duration = Duration::from_secs(150);

/// A save or remove answer names the store it was asked about, and only that one.
fn same_store(value: &Value, store: &Store) -> bool {
    match store {
        Store::Socket(socket) => value["socket"] == *socket,
        Store::Builtin => value["host"] == "pty" && value.get("socket").is_none(),
    }
}

fn execute(core: &Core, job: &Job) -> Result<Outcome, SquadError> {
    let argv = job.argv();
    let args: Vec<_> = argv.iter().map(String::as_str).collect();
    let (value, success) = if let Job::Restore(selection) = job {
        // Core's layout phase has its own 30s budget; permit the result/cleanup.
        // The existing process owner retains its output bound and cancellation.
        let budget = match selection.store {
            Store::Socket(_) => Duration::from_secs(60),
            Store::Builtin => BUILTIN_RESTORE,
        };
        let finished = core.json_finished(&args, budget)?;
        let value = serde_json::from_slice(&finished.stdout).map_err(|_| invalid())?;
        (value, finished.success)
    } else {
        (core.json(&args)?, true)
    };
    if let Some(error) = value.get("error") {
        return match (error["code"].as_str(), error["message"].as_str()) {
            (Some(code), Some(message)) => Err(SquadError::new(code, message)),
            _ => Err(invalid()),
        };
    }
    match job {
        Job::List(store) => {
            let inventory: Inventory = decode(value)?;
            inventory.validate(store.as_ref())?;
            Ok(Outcome::List(inventory))
        }
        Job::Show {
            store: Store::Socket(socket),
            name,
        } => {
            let preview: Preview = decode(value)?;
            preview.validate(socket, name.as_deref())?;
            Ok(Outcome::Show(Box::new(preview)))
        }
        Job::Show {
            store: Store::Builtin,
            name,
        } => {
            let preview: MemberPreview = decode(value)?;
            preview.validate(name.as_deref())?;
            Ok(Outcome::Members(preview))
        }
        Job::Save { store, name } => {
            if value["version"] != 1 || !same_store(&value, store) {
                return Err(invalid());
            }
            let entry: RecoverySnapshotSummary = decode(value)?;
            if entry.name.as_ref() != Some(name) || !valid_digest(&entry.digest) {
                return Err(invalid());
            }
            Ok(Outcome::Saved(entry))
        }
        Job::Remove { store, name } => {
            if value["version"] != 1
                || !same_store(&value, store)
                || value["name"] != *name
                || value["removed"] != true
            {
                return Err(invalid());
            }
            Ok(Outcome::Removed)
        }
        Job::Restore(s) if s.store == Store::Builtin => {
            let restore: MemberRestore = decode(value)?;
            restore.validate(s, success)?;
            Ok(Outcome::MembersRestored(restore))
        }
        Job::Restore(s) => {
            let Store::Socket(socket) = &s.store else {
                unreachable!("built-in restore handled above");
            };
            if !s.layout_only && value.get("revival").is_none() {
                return Err(invalid());
            }
            let restore: Restore = decode(value)?;
            if restore.version != 1
                || restore.socket != *socket
                || restore.name != s.name
                || restore.digest != s.digest
                || restore.layout_only != s.layout_only
                || !matches!(restore.status.as_str(), "completed" | "partial")
                || success != (restore.status == "completed")
                || restore.status == "completed" && !restore.failures.is_empty()
                || !unique_ids(restore.sessions.iter().map(|s| s.recorded.as_str()))
                || !unique_ids(restore.windows.iter().map(|w| w.recorded.as_str()))
                || !unique_ids(restore.panes.iter().map(|p| p.recorded.as_str()))
                || !unique_ids(restore.revival.iter().map(|p| p.recorded.as_str()))
            {
                return Err(invalid());
            }
            Ok(Outcome::Restored(restore))
        }
    }
}

pub(super) fn unique_ids<'a>(mut ids: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    ids.all(|id| !id.is_empty() && seen.insert(id))
}
