//! Finite public-command jobs. No file, host, provider or retry owner here.
use super::model::*;
use crate::core::{Core, SquadError};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub socket: String,
    pub name: Option<String>,
    pub digest: String,
    pub layout_only: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Job {
    List(Option<String>),
    Show {
        socket: String,
        name: Option<String>,
    },
    Save {
        socket: String,
        name: String,
    },
    Remove {
        socket: String,
        name: String,
    },
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
    Saved(Entry),
    Removed,
    Restored(Restore),
}
pub(crate) struct Completed {
    pub task: Task,
    pub(super) result: Result<Outcome, SquadError>,
}
impl Job {
    pub fn argv(&self) -> Vec<String> {
        let (command, socket, name) = match self {
            Self::List(socket) => ("ls", socket.as_deref(), None),
            Self::Show { socket, name } => ("show", Some(socket.as_str()), name.as_deref()),
            Self::Save { socket, name } => ("save", Some(socket.as_str()), Some(name.as_str())),
            Self::Remove { socket, name } => ("rm", Some(socket.as_str()), Some(name.as_str())),
            Self::Restore(s) => ("restore", Some(s.socket.as_str()), s.name.as_deref()),
        };
        let mut argv = vec!["workspace".into(), command.into()];
        if let Some(socket) = socket {
            argv.extend(["--socket".into(), socket.into()]);
        }
        if let Self::Restore(s) = self {
            argv.extend(["--expect".into(), s.digest.clone()]);
            if s.layout_only {
                argv.push("--layout-only".into());
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
fn execute(core: &Core, job: &Job) -> Result<Outcome, SquadError> {
    let argv = job.argv();
    let args: Vec<_> = argv.iter().map(String::as_str).collect();
    let (value, success) = if matches!(job, Job::Restore(_)) {
        // Core's layout phase has its own 30s budget; permit the result/cleanup.
        // The existing process owner retains its output bound and cancellation.
        let finished = core.json_finished(&args, Duration::from_secs(60))?;
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
        Job::List(socket) => {
            let inventory: Inventory = decode(value)?;
            inventory.validate(socket.as_deref())?;
            Ok(Outcome::List(inventory))
        }
        Job::Show { socket, name } => {
            let preview: Preview = decode(value)?;
            preview.validate(socket, name.as_deref())?;
            Ok(Outcome::Show(Box::new(preview)))
        }
        Job::Save { socket, name } => {
            if value["version"] != 1 || value["socket"] != *socket {
                return Err(invalid());
            }
            let entry: Entry = decode(value)?;
            if entry.name.as_ref() != Some(name) || !valid_digest(&entry.digest) {
                return Err(invalid());
            }
            Ok(Outcome::Saved(entry))
        }
        Job::Remove { socket, name } => {
            if value["version"] != 1
                || value["socket"] != *socket
                || value["name"] != *name
                || value["removed"] != true
            {
                return Err(invalid());
            }
            Ok(Outcome::Removed)
        }
        Job::Restore(s) => {
            if !s.layout_only && value.get("revival").is_none() {
                return Err(invalid());
            }
            let restore: Restore = decode(value)?;
            if restore.version != 1
                || restore.socket != s.socket
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

fn unique_ids<'a>(mut ids: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    ids.all(|id| !id.is_empty() && seen.insert(id))
}
