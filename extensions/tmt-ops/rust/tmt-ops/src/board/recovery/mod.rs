//! Saved recovery UI. Presentation owns selection; Core owns every effect.
mod input;
pub(crate) mod load;
mod model;
mod surface;
#[cfg(test)]
pub(in crate::board) mod tests;

use super::picker_surface;
use crate::core::SquadError;
use load::{Job, Outcome, Selection, Store, Task};
use model::{MemberPreview, MemberRestore, Preview, RecoverySnapshotSummary, Restore};
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
    zone: jiff::tz::TimeZone,
    serial: u64,
    screen: Screen,
    /// Fixed by Core's first listing; never chosen here.
    store: Option<Store>,
    /// tmux sockets Core named beside an empty built-in store.
    socket_stores: Vec<String>,
    entries: Vec<RecoverySnapshotSummary>,
    selected: Option<String>,
    /// The tmux preview, or the built-in one: at most one is set, by store.
    preview: Option<Preview>,
    members: Option<MemberPreview>,
    layout_only: bool,
    fresh: bool,
    confirmation: Option<Selection>,
    pending: Option<Task>,
    requested_read: Option<Job>,
    restored: Option<Restore>,
    members_restored: Option<MemberRestore>,
    details: Vec<String>,
    detail_page: usize,
    failure: Option<SquadError>,
    unknown: Option<String>,
    notice: String,
    form_focus: bool,
    /// The Save screen was opened again with a typed name: Discard drops it.
    reopened: bool,
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
    pub fn new(owner: u64, zone: jiff::tz::TimeZone) -> Self {
        Self {
            active: true,
            owner,
            zone,
            serial: 0,
            screen: Screen::List,
            store: None,
            socket_stores: vec![],
            entries: vec![],
            selected: None,
            preview: None,
            members: None,
            layout_only: true,
            fresh: false,
            confirmation: None,
            pending: None,
            requested_read: None,
            restored: None,
            members_restored: None,
            details: vec![],
            detail_page: 0,
            failure: None,
            unknown: None,
            notice: String::new(),
            form_focus: true,
            reopened: false,
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
        if self.restored.is_some() || self.members_restored.is_some() || self.unknown.is_some() {
            self.screen = Screen::Results;
            self.details = match (&self.restored, &self.members_restored) {
                (Some(restored), _) => surface::results(restored),
                (_, Some(restored)) => surface::member_results(restored),
                _ => vec![],
            };
            self.detail_page = 0;
            self.form_focus = true;
            self.invalidate();
            return None;
        }
        if self.pending.as_ref().is_some_and(|t| t.job.writes()) {
            return None;
        }
        // A reopened inner screen shows where it was; Ctrl-R refreshes it. A confirm lost its
        // confirmation on close, so it reopens on the screen that asks for it again.
        match self.screen {
            Screen::Confirm => self.screen = Screen::Preview,
            Screen::Delete => self.screen = Screen::List,
            _ => {}
        }
        if self.screen != Screen::List && !self.entries.is_empty() {
            self.reopened = self.screen == Screen::Save
                && self
                    .form
                    .borrow()
                    .picker
                    .query()
                    .is_some_and(|name| !name.is_empty());
            self.invalidate();
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
        self.members = None;
        self.screen = Screen::List;
        self.read(Job::List(self.store.clone()))
    }
    fn previewed(&self) -> bool {
        self.preview.is_some() || self.members.is_some()
    }
    fn builtin(&self) -> bool {
        self.store == Some(Store::Builtin)
    }
    fn selected_entry(&self) -> Option<&RecoverySnapshotSummary> {
        self.entries
            .iter()
            .find(|e| self.selected.as_deref() == Some(e.id().as_str()))
    }
    fn select(&mut self, id: &str) -> Option<Task> {
        self.confirmation = None;
        self.preview = None;
        self.members = None;
        self.selected = Some(id.into());
        let entry = self.selected_entry()?;
        let job = Job::Show {
            store: self.store.clone()?,
            name: entry.selector().map(str::to_owned),
        };
        self.read(job)
    }
    fn selection(&self) -> Option<Selection> {
        let (store, name, digest) = match (&self.preview, &self.members) {
            (Some(p), _) => (
                Store::Socket(p.snapshot.server.socket.clone()),
                &p.name,
                &p.digest,
            ),
            (_, Some(m)) => (Store::Builtin, &m.name, &m.digest),
            _ => return None,
        };
        if self.selected_entry()?.selector() != name.as_deref() {
            return None;
        }
        let builtin = store == Store::Builtin;
        Some(Selection {
            store,
            name: name.clone(),
            digest: digest.clone(),
            layout_only: !builtin && self.layout_only,
            fresh: builtin && self.fresh,
        })
    }
    pub(in crate::board) fn close(&mut self) {
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
                self.store = Some(inventory.store());
                self.socket_stores = inventory.socket_stores;
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
                self.notice = if !self.builtin() {
                    "No snapshots. A successful Core capture is needed before Save latest capture."
                        .into()
                } else if let Some(socket) = self.socket_stores.first() {
                    // Core chose the built-in store; Ops only says where tmux snapshots are.
                    format!(
                        "No built-in member snapshots. tmux workspace snapshots exist; run tmt workspace ls --socket {} or open Ops inside that tmux server.",
                        crate::launch::shell_word(socket)
                    )
                } else {
                    "No built-in member snapshots yet. Members started with tmt run on the built-in host are recorded automatically.".into()
                };
            }
            Ok(Outcome::Show(preview)) => {
                // Show acquires current bytes; a changed Latest requires a new review.
                if let Some(entry) = self
                    .entries
                    .iter_mut()
                    .find(|e| e.selector() == preview.name.as_deref())
                {
                    entry.digest = preview.digest.clone();
                    entry.captured_at_ms = preview.snapshot.captured_at_ms;
                }
                self.details = surface::topology(&preview);
                self.detail_page = 0;
                self.preview = Some(*preview);
                self.members = None;
            }
            Ok(Outcome::Members(preview)) => {
                if let Some(entry) = self
                    .entries
                    .iter_mut()
                    .find(|e| e.selector() == preview.name.as_deref())
                {
                    entry.digest = preview.digest.clone();
                    entry.captured_at_ms = preview.captured_at_ms;
                }
                self.details = surface::members(&preview);
                self.detail_page = 0;
                self.members = Some(preview);
                self.preview = None;
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
                self.members_restored = None;
                self.preview = None;
                self.unknown = None;
                self.notice = "Created resources remain. Startup requested is not conversation confirmed; nothing is rolled back or retried.".into();
            }
            Ok(Outcome::MembersRestored(result)) => {
                self.screen = Screen::Results;
                self.details = surface::member_results(&result);
                self.detail_page = 0;
                self.form_focus = true;
                self.members_restored = Some(result);
                self.restored = None;
                self.members = None;
                self.unknown = None;
                self.notice = "Started means the member was asked to resume; running members were left alone. Nothing is retried automatically.".into();
            }
            Err(error) => {
                if let Job::Restore(selection) = &completed.task.job {
                    let guarded = matches!(
                        error.code.as_str(),
                        "WORKSPACE_SNAPSHOT_CHANGED" | "WORKSPACE_SNAPSHOT_NOT_FOUND"
                    );
                    self.preview = None;
                    self.members = None;
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
                        self.members_restored = None;
                        self.form_focus = true;
                        self.unknown = Some(format!(
                            "{}: {}. {} before deciding any further action; no automatic retry.",
                            error.code,
                            error.message,
                            if selection.store == Store::Builtin {
                                "Check which members are running"
                            } else {
                                "Inspect the original socket and live resources"
                            }
                        ));
                    }
                    // One fresh read after a restore failure. A missing Latest
                    // shares a Core error code with other failures, so retain
                    // Unknown rather than infer whether effects were possible.
                    let next = self.active.then(|| {
                        self.issue(Job::Show {
                            store: selection.store.clone(),
                            name: selection.name.clone(),
                        })
                    });
                    self.failure = Some(error);
                    return next;
                }
                if matches!(completed.task.job, Job::Show { .. }) {
                    self.preview = None;
                    self.members = None;
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
                self.reopened = false;
                self.screen = Screen::List;
            }
            "discard" if self.screen == Screen::Save => {
                self.reopened = false;
                self.screen = Screen::List;
                self.form = RefCell::new(picker_surface::State::new(
                    Some(String::new()),
                    vec![],
                    None,
                ));
            }
            "details" if self.previewed() => {
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
                if self.builtin() {
                    self.fresh = !self.fresh;
                } else {
                    self.layout_only = !self.layout_only;
                }
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
                    self.members_restored = None;
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
            "save" if self.store.is_some() && self.entries.iter().any(|e| e.name.is_none()) => {
                self.screen = Screen::Save;
                self.form_focus = true;
                self.reopened = false;
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
                } else if let Some(store) = self.store.clone() {
                    return Some(self.issue(Job::Save { store, name }));
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
                    store: self.store.clone()?,
                    name,
                }));
            }
            id if id.starts_with("snapshot:") => {
                if self.selected.as_deref() != Some(id) {
                    return self.select(id);
                }
                if self.previewed() {
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
        use crate::board::app::Effect;
        if self.input.is_some()
            || self.searching
            || self
                .workspace_form
                .as_ref()
                .is_some_and(|draft| draft.shown)
            || self.modal_open() && self.menu.is_none()
        {
            return self.say("Open snapshots after closing the current input or overlay.");
        }
        self.menu = None;
        // This controller is never replaced while this App lives. Its frozen
        // socket and submitted results survive closing the presentation.
        let controller = self.recovery.get_or_insert_with(|| {
            Controller::new(
                1,
                self.local_zone
                    .clone()
                    .unwrap_or_else(jiff::tz::TimeZone::system),
            )
        });
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
