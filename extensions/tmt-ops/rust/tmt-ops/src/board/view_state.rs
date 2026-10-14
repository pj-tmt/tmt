//! A disposable, per-user cold-start view hint. The existing lane publishes it;
//! the board only projects UUIDs and offers its latest complete view on the tick.
use super::{app::App, app::RowTarget, resume, shell::OpsTab, workspace::OpsTarget};
use crate::cache;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const VERSION: u64 = 1;
const MAX_BYTES: u64 = 64 * 1024;
const MAX_ROOMS: usize = 1024;
const SAVE_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ViewState {
    pub shell: OpsTab,
    pub expanded: BTreeSet<String>,
    pub pool_expanded: bool,
    pub selected: Option<OpsTarget>,
}

impl ViewState {
    fn read(path: &Path) -> Option<Self> {
        Self::decode(&serde_json::from_slice(&cache::read_regular(path, MAX_BYTES)?).ok()?)
    }

    fn decode(document: &Value) -> Option<Self> {
        if document["version"].as_u64()? != VERSION
            || document["squadsExpanded"].as_array()?.len() > MAX_ROOMS
        {
            return None;
        }
        Some(Self {
            shell: resume::shell_from(&document["shell"])?,
            expanded: resume::room_ids(&document["squadsExpanded"])?
                .into_iter()
                .collect(),
            pool_expanded: document["poolExpanded"].as_bool()?,
            selected: match document.get("selected")? {
                Value::Null => None,
                value => match resume::row_from(value)? {
                    RowTarget::Workspace(target) => Some(target),
                    _ => return None,
                },
            },
        })
    }

    fn bytes(&self) -> Option<Vec<u8>> {
        if self.expanded.len() > MAX_ROOMS {
            return None;
        }
        let document = json!({
            "version": VERSION,
            "shell": resume::shell_json(self.shell),
            "squadsExpanded": self.expanded,
            "poolExpanded": self.pool_expanded,
            "selected": self.selected.clone().map(RowTarget::Workspace).as_ref().map(resume::row_json),
        });
        // Runtime observations cannot publish a hint its own bounded decoder rejects.
        if Self::decode(&document).as_ref() != Some(self) {
            return None;
        }
        let bytes = serde_json::to_vec(&document).ok()?;
        (bytes.len() as u64 <= MAX_BYTES).then_some(bytes)
    }

    fn capture(app: &App) -> Option<Self> {
        let snapshot = app.workspace_snapshot()?;
        if app.loading()
            || app.loading_since.is_some()
            || app.error.is_some()
            || !snapshot.roster.complete()
        {
            return None;
        }
        Some(Self {
            shell: app.shell.tab,
            expanded: app
                .workspace
                .expanded
                .iter()
                .filter(|id| {
                    snapshot
                        .roster
                        .squads
                        .iter()
                        .any(|room| room.room_id == **id)
                })
                .cloned()
                .collect(),
            pool_expanded: app.workspace.pool_expanded,
            selected: app
                .workspace
                .selected
                .clone()
                .filter(|target| snapshot.roster.contains(target)),
        })
    }

    fn reconcile(&mut self, app: &App) {
        let roster = &app
            .workspace_snapshot()
            .expect("complete view checked by the caller")
            .roster;
        self.expanded
            .retain(|id| roster.squads.iter().any(|room| room.room_id == *id));
        self.selected = self
            .selected
            .take()
            .filter(|target| roster.contains(target));
    }
}

/// One queue slot for the latest projection, not a queue of cursor movements.
#[derive(Debug, Clone)]
pub(super) struct Writer(Arc<Mutex<Pending>>);

#[derive(Debug)]
struct Pending {
    path: PathBuf,
    latest: Option<ViewState>,
    saved: Option<ViewState>,
    attempted: Option<Instant>,
    queued: bool,
}

impl Writer {
    fn new(path: PathBuf, saved: Option<ViewState>) -> Self {
        Self(Arc::new(Mutex::new(Pending {
            path,
            latest: saved.clone(),
            saved,
            attempted: None,
            queued: false,
        })))
    }

    fn offer(&self, state: ViewState, now: Instant) -> bool {
        let Ok(mut pending) = self.0.lock() else {
            return false;
        };
        pending.latest = Some(state);
        if pending.latest == pending.saved
            || pending.queued
            || pending
                .attempted
                .is_some_and(|at| now.saturating_duration_since(at) < SAVE_EVERY)
        {
            return false;
        }
        pending.queued = true;
        true
    }

    /// Called only by the existing lane, or once after it has joined on exit.
    /// No mutex is held across disk I/O, so input never waits for publication.
    pub fn publish(&self, now: Instant, flush: bool) {
        self.publish_with(now, flush, cache::replace);
    }

    fn publish_with(
        &self,
        now: Instant,
        flush: bool,
        replace: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
    ) {
        let (path, state) = {
            let Ok(mut pending) = self.0.lock() else {
                return;
            };
            if pending.latest == pending.saved
                || (!flush
                    && pending
                        .attempted
                        .is_some_and(|at| now.saturating_duration_since(at) < SAVE_EVERY))
            {
                pending.queued = false;
                return;
            }
            let Some(state) = pending.latest.clone() else {
                pending.queued = false;
                return;
            };
            pending.attempted = Some(now);
            (pending.path.clone(), state)
        };
        let result = state
            .bytes()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))
            .and_then(|bytes| replace(&path, &bytes));
        if let Ok(mut pending) = self.0.lock() {
            if result.is_ok() {
                pending.saved = Some(state);
            }
            // A failure keeps the latest projection for a later bounded tick.
            pending.queued = false;
        }
    }
}

/// Owned by the interactive entry; pure App constructors never resolve a path.
pub(super) struct Session {
    hint: Option<ViewState>,
    explicit_startup: bool,
    writer: Writer,
}

impl Session {
    pub fn load(path: PathBuf, exec_resume: bool, explicit_startup: bool) -> Self {
        let saved = ViewState::read(&path);
        Self {
            hint: if exec_resume { None } else { saved.clone() },
            explicit_startup,
            writer: Writer::new(path, saved),
        }
    }

    fn prepare(&mut self, app: &mut App) -> Option<(bool, ViewState)> {
        // A partial or failed first view keeps the hint pending and cannot
        // replace a previously valid file with the loading defaults.
        let current = ViewState::capture(app)?;
        if let Some(mut hint) = self.hint.take() {
            hint.reconcile(app);
            app.carry_view_hint(&hint, self.explicit_startup);
            Some((true, ViewState::capture(app)?))
        } else {
            Some((false, current))
        }
    }

    pub fn tick(&mut self, app: &mut App, now: Instant) -> (bool, Option<Writer>) {
        let Some((restored, state)) = self.prepare(app) else {
            return (false, None);
        };
        let job = self.writer.offer(state, now).then(|| self.writer.clone());
        (restored, job)
    }

    /// A single final flush, after the terminal is restored and lane joined.
    pub fn flush(&mut self, app: &mut App, now: Instant) {
        if let Some((_, state)) = self.prepare(app) {
            self.writer.offer(state, now);
            self.writer.publish(now, true);
        }
    }
}

#[cfg(test)]
mod tests;
