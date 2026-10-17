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

/// A layer name is admitted before it can become a cache path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layer(String);
impl Default for Layer {
    fn default() -> Self {
        Self("default".into())
    }
}
impl Layer {
    pub(crate) fn parse(name: &str) -> Result<Self, String> {
        if !(1..=32).contains(&name.len())
            || !name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("View name must be 1–32 lowercase letters, digits or hyphens.".into());
        }
        Ok(Self(name.into()))
    }
    pub(crate) fn name(&self) -> &str {
        &self.0
    }
    pub(super) fn path(&self, directory: &Path) -> PathBuf {
        if self == &Self::default() {
            directory.join("view.json")
        } else {
            directory.join("views").join(format!("{}.json", self.0))
        }
    }
}

/// Reset is an explicit interactive entry operation, never a paint or exec effect.
fn reset(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::board) struct Order {
    pub agents: crate::config::OpsSort,
    pub reversed: bool,
    pub overview: Option<crate::config::OverviewSort>,
}
impl From<&crate::config::OpsPreferences> for Order {
    fn from(preferences: &crate::config::OpsPreferences) -> Self {
        Self {
            agents: preferences.sort,
            reversed: preferences.reversed,
            overview: preferences.overview_sort,
        }
    }
}

/// Committed view controls are separate from shared Config preferences and drafts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Controls {
    ratios: super::pane_resize::Ratios,
    query: String,
    agents_filter: super::workspace::AgentsFilter,
    overview_view: super::overview::monitor::View,
    quota_window: super::limits::QuotaWindow,
    order: Order,
}
impl Controls {
    pub(super) fn capture(app: &App) -> Self {
        Self {
            ratios: app.pane_resize.ratios(),
            query: app.search.clone(),
            agents_filter: app.workspace.filter,
            overview_view: app.overview_view,
            quota_window: app.usage_chart.borrow().quota_window(),
            order: app.workspace_order(),
        }
    }
    pub(super) fn apply(&self, app: &mut App) {
        app.pane_resize.restore(self.ratios.clone());
        app.search = self.query.clone();
        app.workspace.query = self.query.clone();
        app.workspace.filter = self.agents_filter;
        app.overview_view = self.overview_view;
        app.usage_chart
            .borrow_mut()
            .restore_quota_window(self.quota_window);
        app.workspace.order = Some(self.order);
        app.overview_rows.borrow_mut().clear();
    }
    pub(super) fn json(&self) -> Value {
        use super::workspace::AgentsFilter;
        use crate::config::{OpsSort, OverviewDirection};
        json!({ "paneRatios": super::pane_resize::ratios_json(&self.ratios), "query": self.query,
            "agentsFilter": match self.agents_filter { AgentsFilter::All => "all", AgentsFilter::Pinned => "pinned" },
            "overviewView": match self.overview_view { super::overview::monitor::View::Squads => "squads", super::overview::monitor::View::Agents => "agents" },
            "quotaWindow": self.quota_window.key(),
            "sort": match self.order.agents { OpsSort::Name => "name", OpsSort::Attention => "attention", OpsSort::Members => "members" },
            "reversed": self.order.reversed,
            "overviewSort": self.order.overview.map(|order| json!({"column":order.column.key(), "descending":order.direction == OverviewDirection::Descending})) })
    }
    pub(super) fn decode(value: &Value) -> Option<Self> {
        use super::workspace::AgentsFilter;
        use crate::config::{OpsSort, OverviewColumn, OverviewDirection, OverviewSort};
        let query = value["query"].as_str()?;
        if query.len() > super::search_field::MAX_BYTES || query.chars().any(char::is_control) {
            return None;
        }
        Some(Self {
            ratios: super::pane_resize::ratios_decode(value.get("paneRatios"))?,
            query: query.into(),
            agents_filter: match value["agentsFilter"].as_str()? {
                "all" => AgentsFilter::All,
                "needs-you" => AgentsFilter::All,
                "pinned" => AgentsFilter::Pinned,
                _ => return None,
            },
            overview_view: match value.get("overviewView").map(Value::as_str) {
                Some(Some("squads")) => super::overview::monitor::View::Squads,
                None | Some(Some("agents")) => super::overview::monitor::View::Agents,
                _ => return None,
            },
            quota_window: value
                .get("quotaWindow")
                .and_then(Value::as_str)
                .and_then(super::limits::QuotaWindow::parse)
                .unwrap_or_default(),
            order: Order {
                agents: match value["sort"].as_str()? {
                    "name" => OpsSort::Name,
                    "attention" => OpsSort::Attention,
                    "members" => OpsSort::Members,
                    _ => return None,
                },
                reversed: value["reversed"].as_bool()?,
                overview: match value.get("overviewSort")? {
                    Value::Null => None,
                    order => Some(OverviewSort {
                        column: OverviewColumn::ALL.into_iter().find(|column| {
                            column.key() == order["column"].as_str().unwrap_or("")
                        })?,
                        direction: if order["descending"].as_bool()? {
                            OverviewDirection::Descending
                        } else {
                            OverviewDirection::Ascending
                        },
                    }),
                },
            },
        })
    }
}

/// Only known, unique pane offsets fit the bounded cache document.
pub(super) fn scrolls_decode(value: &Value) -> Option<Vec<(crate::config::Pane, usize)>> {
    let rows = value.as_array()?;
    if rows.len() > 32 {
        return None;
    }
    let mut seen = BTreeSet::new();
    rows.iter()
        .map(|row| {
            let pair = row.as_array()?;
            if pair.len() != 2 {
                return None;
            }
            let pane = crate::config::Pane::parse(pair[0].as_str()?)?;
            if !seen.insert(pane) {
                return None;
            }
            Some((pane, usize::try_from(pair[1].as_u64()?).ok()?))
        })
        .collect()
}

const VERSION: u64 = 1;
const MAX_BYTES: u64 = 64 * 1024;
const MAX_ROOMS: usize = 1024;
const SAVE_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ViewState {
    pub shell: OpsTab,
    pub controls: Option<Controls>,
    pub scrolls: Vec<(crate::config::Pane, usize)>,
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
            controls: match document.get("controls") {
                None | Some(Value::Null) => None,
                Some(value) => Some(Controls::decode(value)?),
            },
            scrolls: match document.get("scrolls") {
                None => Vec::new(),
                Some(value) => scrolls_decode(value)?,
            },
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
            "controls": self.controls.as_ref().map(Controls::json),
            "scrolls": self.scrolls.iter().map(|(pane, offset)| json!([pane.title(), offset])).collect::<Vec<_>>(),
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
        if app.searching
            || app.loading()
            || app.loading_since.is_some()
            || app.error.is_some()
            || !snapshot.roster.complete()
        {
            return None;
        }
        Some(Self {
            shell: app.shell.tab,
            controls: Some(Controls::capture(app)),
            scrolls: app.scrolls.offsets(),
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
    pub fn open(
        path: PathBuf,
        exec_resume: bool,
        explicit_startup: bool,
        reset_view: bool,
    ) -> io::Result<Self> {
        if reset_view && !exec_resume {
            reset(&path)?;
        }
        Ok(Self::load(path, exec_resume, explicit_startup))
    }

    fn load(path: PathBuf, exec_resume: bool, explicit_startup: bool) -> Self {
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
