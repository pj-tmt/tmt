//! Saved recovery UI. Presentation owns selection; Core owns every effect.
mod input;
pub(crate) mod load;
mod model;
mod surface;
#[cfg(test)]
mod tests;

use super::picker_surface;
use crate::core::SquadError;
use load::{Job, Outcome, Selection, Task};
use model::{Preview, RecoverySnapshotSummary, Restore};
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    List,
    Preview,
    Confirm,
    Save,
    Delete,
    Results,
}
pub(in crate::board) struct Controller {
    pub active: bool,
    owner: u64,
    serial: u64,
    screen: Screen,
    socket: Option<String>,
    entries: Vec<RecoverySnapshotSummary>,
    selected: Option<String>,
    preview: Option<Preview>,
    layout_only: bool,
    confirmation: Option<Selection>,
    pending: Option<Task>,
    requested_read: Option<Job>,
    restored: Option<Restore>,
    details: Vec<String>,
    detail_page: usize,
    failure: Option<SquadError>,
    unknown: Option<String>,
    notice: String,
    form_focus: bool,
    readable: Cell<bool>,
    list: RefCell<picker_surface::State>,
    panel: RefCell<picker_surface::State>,
    form: RefCell<picker_surface::State>,
}
impl Controller {
    pub(in crate::board) fn owns_typing(&self) -> bool {
        self.active
            && self.screen == Screen::Save
            && self.form_focus
            && !self.pending.as_ref().is_some_and(|task| task.job.writes())
    }
    pub fn new(owner: u64) -> Self {
        Self {
            active: true,
            owner,
            serial: 0,
            screen: Screen::List,
            socket: None,
            entries: vec![],
            selected: None,
            preview: None,
            layout_only: true,
            confirmation: None,
            pending: None,
            requested_read: None,
            restored: None,
            details: vec![],
            detail_page: 0,
            failure: None,
            unknown: None,
            notice: String::new(),
            form_focus: true,
            readable: Cell::new(false),
            list: RefCell::new(picker_surface::State::new(None, vec![], None)),
            panel: RefCell::new(picker_surface::State::new(None, vec![], None)),
            form: RefCell::new(picker_surface::State::new(
                Some(String::new()),
                vec![],
                None,
            )),
        }
    }
    pub fn open(&mut self) -> Option<Task> {
        self.active = true;
        // A report survives closing its view. Reopening never replays the write.
        if self.restored.is_some() || self.unknown.is_some() {
            self.screen = Screen::Results;
            self.details = self
                .restored
                .as_ref()
                .map(surface::results)
                .unwrap_or_default();
            self.detail_page = 0;
            self.form_focus = true;
            self.invalidate();
            return None;
        }
        if self.pending.as_ref().is_some_and(|t| t.job.writes()) {
            return None;
        }
        self.refresh()
    }
    pub fn blocks_reload(&self) -> bool {
        self.active || self.pending.is_some() || self.unknown.is_some()
    }
    fn issue(&mut self, job: Job) -> Task {
        self.serial += 1;
        let task = Task {
            owner: self.owner,
            serial: self.serial,
            job,
        };
        self.pending = Some(task.clone());
        self.failure = None;
        self.invalidate();
        task
    }
    fn read(&mut self, job: Job) -> Option<Task> {
        debug_assert!(!job.writes());
        if self.pending.is_some() {
            // One active read and the latest requested selection, never a queue
            // of obsolete reads behind rapid navigation or repeated refresh.
            self.requested_read = Some(job);
            None
        } else {
            Some(self.issue(job))
        }
    }
    fn refresh(&mut self) -> Option<Task> {
        self.confirmation = None;
        self.preview = None;
        self.screen = Screen::List;
        self.read(Job::List(self.socket.clone()))
    }
    fn selected_entry(&self) -> Option<&RecoverySnapshotSummary> {
        self.entries
            .iter()
            .find(|e| self.selected.as_deref() == Some(e.id().as_str()))
    }
    fn select(&mut self, id: &str) -> Option<Task> {
        self.confirmation = None;
        self.preview = None;
        self.selected = Some(id.into());
        let entry = self.selected_entry()?;
        let job = Job::Show {
            socket: self.socket.clone()?,
            name: entry.name.clone(),
        };
        self.read(job)
    }
    fn selection(&self) -> Option<Selection> {
        let p = self.preview.as_ref()?;
        if self.selected_entry()?.name != p.name {
            return None;
        }
        Some(Selection {
            socket: p.snapshot.server.socket.clone(),
            name: p.name.clone(),
            digest: p.digest.clone(),
            layout_only: self.layout_only,
        })
    }
    fn close(&mut self) {
        self.active = false;
        self.confirmation = None;
        self.requested_read = None;
        // Keep ownership of the active read until its completion. Reopening
        // coalesces behind it instead of queuing abandoned reads on the lane.
        self.invalidate();
    }
    pub fn invalidate(&self) {
        self.readable.set(false);
        for state in [&self.list, &self.panel, &self.form] {
            state.borrow_mut().invalidate();
        }
    }
    pub fn loaded(&mut self, completed: load::Completed) -> Option<Task> {
        if self.pending.as_ref() != Some(&completed.task) {
            return None;
        }
        self.pending = None;
        if !self.active && !completed.task.job.writes() {
            self.requested_read = None;
            self.invalidate();
            return None;
        }
        if let Some(job) = self.requested_read.take() {
            return Some(self.issue(job));
        }
        self.confirmation = None;
        self.failure = None;
        self.invalidate();
        match completed.result {
            Ok(Outcome::List(inventory)) => {
                self.socket = Some(inventory.socket);
                self.entries = inventory.snapshots;
                let id = self
                    .selected
                    .as_ref()
                    .filter(|id| self.entries.iter().any(|e| e.id() == **id))
                    .cloned()
                    .or_else(|| self.entries.first().map(RecoverySnapshotSummary::id));
                if let Some(id) = id {
                    let rows = self
                        .rows()
                        .into_iter()
                        .map(|r| tmt_tui::components::ListRow {
                            id: r.id,
                            disabled: r.disabled,
                        })
                        .collect();
                    self.list.borrow_mut().reconcile(rows);
                    self.list.borrow_mut().select(&id);
                    return self.select(&id);
                }
                self.selected = None;
                self.notice =
                    "No snapshots. A successful Core capture is needed before Save latest capture."
                        .into();
            }
            Ok(Outcome::Show(preview)) => {
                // Show acquires current bytes; a changed Latest requires a new review.
                if let Some(entry) = self.entries.iter_mut().find(|e| e.name == preview.name) {
                    entry.digest = preview.digest.clone();
                    entry.captured_at_ms = preview.snapshot.captured_at_ms;
                }
                self.details = surface::topology(&preview);
                self.detail_page = 0;
                self.preview = Some(*preview);
            }
            Ok(Outcome::Saved(entry)) => {
                self.notice = format!(
                    "Saved {} from the published capture; capture time is unchanged.",
                    entry.label()
                );
                self.selected = Some(entry.id());
                return self.refresh();
            }
            Ok(Outcome::Removed) => {
                self.notice =
                    "Named snapshot deleted. Latest and live resources are unchanged.".into();
                self.selected = None;
                return self.refresh();
            }
            Ok(Outcome::Restored(result)) => {
                self.screen = Screen::Results;
                self.details = surface::results(&result);
                self.detail_page = 0;
                self.form_focus = true;
                self.restored = Some(result);
                self.preview = None;
                self.unknown = None;
                self.notice = "Created resources remain. Startup requested is not conversation confirmed; nothing is rolled back or retried.".into();
            }
            Err(error) => {
                if let Job::Restore(selection) = &completed.task.job {
                    let guarded = matches!(
                        error.code.as_str(),
                        "WORKSPACE_SNAPSHOT_CHANGED" | "WORKSPACE_SNAPSHOT_NOT_FOUND"
                    );
                    self.preview = None;
                    self.details.clear();
                    self.detail_page = 0;
                    if guarded {
                        self.notice = format!(
                            "{} Not applied: {}. Review a fresh preview before restoring.",
                            tmt_cli_style::mark::Mark::Offline.symbol(),
                            error.message
                        );
                        self.screen = Screen::List;
                    } else {
                        self.screen = Screen::Results;
                        self.restored = None;
                        self.form_focus = true;
                        self.unknown = Some(format!(
                            "{}: {}. Inspect the original socket and live resources before deciding any further action; no automatic retry.",
                            error.code, error.message
                        ));
                    }
                    // One fresh read after a restore failure. A missing Latest
                    // shares a Core error code with other failures, so retain
                    // Unknown rather than infer whether effects were possible.
                    let next = self.active.then(|| {
                        self.issue(Job::Show {
                            socket: selection.socket.clone(),
                            name: selection.name.clone(),
                        })
                    });
                    self.failure = Some(error);
                    return next;
                }
                if matches!(completed.task.job, Job::Show { .. }) {
                    self.preview = None;
                    if self.screen != Screen::Results {
                        self.screen = Screen::List;
                    }
                }
                // A failed fresh Show never schedules another Show.
                self.failure = Some(error);
            }
        }
        None
    }
    fn choose(&mut self, id: &str) -> Option<Task> {
        if id != "close" && self.pending.as_ref().is_some_and(|t| t.job.writes()) {
            return None;
        }
        match id {
            "close" => self.close(),
            "refresh" => return self.refresh(),
            "back" | "cancel" => {
                self.confirmation = None;
                self.screen = Screen::List;
            }
            "details" if self.preview.is_some() => {
                self.screen = Screen::Preview;
                self.form_focus = true;
            }
            "previous-details" | "next-details" => {
                let pages = self.details.len().div_ceil(surface::PAGE_SIZE).max(1);
                self.detail_page = if id == "previous-details" {
                    self.detail_page.saturating_sub(1)
                } else {
                    (self.detail_page + 1).min(pages - 1)
                };
                self.panel = RefCell::new(picker_surface::State::new(None, vec![], None));
                self.form_focus = true;
            }
            "mode" => {
                self.layout_only = !self.layout_only;
                self.confirmation = None;
            }
            "review" if self.pending.is_none() => {
                self.confirmation = self.selection();
                if self.confirmation.is_some() {
                    self.screen = Screen::Confirm;
                    self.panel = RefCell::new(picker_surface::State::new(None, vec![], None));
                }
            }
            "confirm"
                if self.screen == Screen::Confirm
                    && self.pending.is_none()
                    && self.readable.get() =>
            {
                if self.confirmation.as_ref() == self.selection().as_ref()
                    && let Some(selection) = self.confirmation.take()
                {
                    self.screen = Screen::Results;
                    self.restored = None;
                    self.details.clear();
                    self.detail_page = 0;
                    self.form_focus = true;
                    self.unknown = None;
                    return Some(self.issue(Job::Restore(selection)));
                }
            }
            "confirm" if self.screen == Screen::Confirm => {
                self.notice = "Resize or scroll back until the complete target, mode and both Cancel/Confirm choices are visible.".into();
            }
            "save" if self.socket.is_some() && self.entries.iter().any(|e| e.name.is_none()) => {
                self.screen = Screen::Save;
                self.form_focus = true;
                self.form = RefCell::new(picker_surface::State::new(
                    Some(String::new()),
                    vec![],
                    None,
                ));
            }
            "save-confirm" if self.screen == Screen::Save && self.pending.is_none() => {
                let name = self
                    .form
                    .borrow()
                    .picker
                    .query()
                    .unwrap_or_default()
                    .to_owned();
                if !model::valid_name(&name) {
                    self.notice = "Use 1–64 lowercase letters, digits, _ or -, starting with a letter or digit; latest is reserved.".into();
                } else if let Some(socket) = self.socket.clone() {
                    return Some(self.issue(Job::Save { socket, name }));
                }
            }
            "delete" if self.selected_entry().is_some_and(|e| e.name.is_some()) => {
                self.screen = Screen::Delete;
                self.panel = RefCell::new(picker_surface::State::new(None, vec![], None));
            }
            "delete-confirm"
                if self.screen == Screen::Delete
                    && self.pending.is_none()
                    && self.readable.get() =>
            {
                let name = self.selected_entry()?.name.clone()?;
                return Some(self.issue(Job::Remove {
                    socket: self.socket.clone()?,
                    name,
                }));
            }
            id if id.starts_with("snapshot:") => {
                if self.selected.as_deref() != Some(id) {
                    return self.select(id);
                }
                if self.preview.is_some() {
                    self.screen = Screen::Preview;
                    self.form_focus = true;
                }
            }
            _ => {}
        }
        self.invalidate();
        None
    }
}

impl crate::board::app::App {
    pub(in crate::board) fn recovery_shown(&self) -> bool {
        self.recovery.as_ref().is_some_and(|c| c.active)
    }
    pub(in crate::board) fn open_recovery(&mut self) -> crate::board::app::Effect {
        use crate::board::{app::Effect, shell::OpsTab};
        if self.shell.tab != OpsTab::Agents
            || self.input.is_some()
            || self.searching
            || self
                .workspace_form
                .as_ref()
                .is_some_and(|draft| draft.shown)
            || self.modal_open() && self.menu.is_none()
        {
            return self.say(
                "Open snapshots from the Agents list after closing the current input or overlay.",
            );
        }
        self.menu = None;
        // This controller is never replaced while this App lives. Its frozen
        // socket and submitted results survive closing the presentation.
        let controller = self.recovery.get_or_insert_with(|| Controller::new(1));
        controller.open().map_or(Effect::None, Effect::Recovery)
    }
    pub(in crate::board) fn recovery_event(
        &mut self,
        event: &ratatui::crossterm::event::Event,
    ) -> Option<crate::board::app::Effect> {
        use crate::board::app::Effect;
        let task = self.recovery.as_mut()?.input(event);
        Some(task.map_or(Effect::None, Effect::Recovery))
    }
    pub(in crate::board) fn finished_recovery(
        &mut self,
        completed: load::Completed,
    ) -> crate::board::app::Effect {
        use crate::board::app::Effect;
        self.recovery
            .as_mut()
            .and_then(|c| c.loaded(completed))
            .map_or(Effect::None, Effect::Recovery)
    }
}
