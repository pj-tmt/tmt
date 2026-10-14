//! Explicit discovery presentation. Core alone observes and admits a binding.
mod input;
pub(crate) mod load;
mod model;
mod surface;
#[cfg(test)]
mod tests;

use super::picker_surface;
use crate::core::SquadError;
use load::{Job, Outcome, ScanScope, Task};
use model::{AgentSelector, BindReport, BindSelection, PaneKey, ScanObservation};
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Intro,
    Candidates,
    Details,
    Form,
    Confirm,
    Results,
}

impl super::app::App {
    pub(in crate::board) fn discovery_shown(&self) -> bool {
        self.discovery.as_ref().is_some_and(|c| c.active)
    }
    pub(in crate::board) fn open_discovery(&mut self) -> super::app::Effect {
        use super::{app::Effect, shell::OpsTab};
        use crate::config::Pane;
        if self.shell.tab != OpsTab::Agents
            || self.focused_pane() == Some(Pane::Replies)
            || self.input.is_some()
            || self.searching
            || self.modal_open() && self.menu.is_none()
        {
            return self.say(
                "Open Add agent from the Agents list after closing the current input or overlay.",
            );
        }
        let known: Vec<_> = self
            .workspace_snapshot()
            .map(|s| {
                s.roster
                    .members
                    .values()
                    .filter(|a| model::canonical_uuid(&a.id))
                    .map(|a| model::ExistingAgentEvidence {
                        id: a.id.clone(),
                        name: a.name.clone(),
                        binding: None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.menu = None;
        let controller = self.discovery.get_or_insert_with(|| {
            Controller::new(
                1,
                ScanScope {
                    host: None,
                    socket: crate::effects::tmux_socket(),
                },
            )
        });
        controller.known = known;
        controller.open();
        Effect::None
    }
    pub(in crate::board) fn discovery_event(
        &mut self,
        event: &ratatui::crossterm::event::Event,
    ) -> Option<super::app::Effect> {
        use super::app::Effect;
        let task = self.discovery.as_mut()?.input(event);
        Some(task.map_or(Effect::None, Effect::Discovery))
    }
    pub(in crate::board) fn finished_discovery(
        &mut self,
        completed: load::Completed,
    ) -> super::app::Effect {
        use super::app::Effect;
        let Some(controller) = &mut self.discovery else {
            return Effect::None;
        };
        let completion = controller.loaded(completed);
        if let Some(id) = completion.bound_uuid {
            self.discovered_uuid = Some(id);
            return Effect::Refresh;
        }
        completion.task.map_or(Effect::None, Effect::Discovery)
    }
    /// Only a newly accepted public refresh may expose the bound UUID.
    pub(in crate::board) fn discovery_refreshed(&mut self) {
        use super::{
            shell::OpsTab,
            workspace::{OpsTarget, OpsWorkspace},
        };
        let Some(id) = self.discovered_uuid.as_ref() else {
            return;
        };
        let Some(snapshot) = self.workspace_snapshot() else {
            return;
        };
        // Reuse the roster projection's membership/unknown rules. A partial
        // refresh may know the identity without proving its visible location.
        let expanded = OpsWorkspace {
            pool_expanded: true,
            expanded: snapshot
                .roster
                .squads
                .iter()
                .map(|s| s.room_id.clone())
                .collect(),
            ..Default::default()
        };
        let Some(target) = expanded
            .rows(&snapshot.roster, &snapshot.preferences)
            .into_iter()
            .find_map(|row| match &row.target {
                OpsTarget::Member { identity_id, .. } if identity_id == id => Some(row.target),
                _ => None,
            })
        else {
            return;
        };
        self.discovered_uuid = None;
        if let OpsTarget::Member { room_id, .. } = &target {
            if let Some(room) = room_id {
                self.workspace.expanded.insert(room.clone());
            } else {
                self.workspace.pool_expanded = true;
            }
        }
        self.search.clear();
        self.workspace.selected = Some(target);
        self.shell.tab = OpsTab::Agents;
        self.focus = 0;
        self.workspace_reconcile();
        self.invalidate_frame_geometry();
    }
}
#[derive(Clone, Debug)]
struct Attempt {
    selection: BindSelection,
    result: Result<BindReport, SquadError>,
}
pub(in crate::board) struct Controller {
    pub active: bool,
    owner: u64,
    serial: u64,
    screen: Screen,
    scope: ScanScope,
    pending: Option<Task>,
    requested_scan: Option<ScanScope>,
    observation: Option<ScanObservation>,
    selected: Option<PaneKey>,
    confirmation: Option<BindSelection>,
    attempts: Vec<Attempt>,
    failure: Option<SquadError>,
    readable: Cell<bool>,
    ui: RefCell<picker_surface::State>,
    form_focus: bool,
    existing: bool,
    save: bool,
    known: Vec<model::ExistingAgentEvidence>,
    result_index: usize,
    editor: tmt_tui::components::Editor,
    editor_area: Cell<Option<ratatui::layout::Rect>>,
    editor_offset: Cell<usize>,
}
pub(super) struct Completion {
    pub task: Option<Task>,
    pub bound_uuid: Option<String>,
}
impl Controller {
    pub fn new(owner: u64, scope: ScanScope) -> Self {
        Self {
            active: true,
            owner,
            serial: 0,
            screen: Screen::Intro,
            scope,
            pending: None,
            requested_scan: None,
            observation: None,
            selected: None,
            confirmation: None,
            attempts: vec![],
            failure: None,
            readable: Cell::new(false),
            ui: RefCell::new(picker_surface::State::new(None, vec![], None)),
            form_focus: true,
            existing: false,
            save: false,
            known: vec![],
            result_index: 0,
            editor: tmt_tui::components::Editor::default(),
            editor_area: Cell::new(None),
            editor_offset: Cell::new(0),
        }
    }
    pub fn open(&mut self) {
        self.active = true;
        self.changed();
    }
    pub fn reload_blocked(&self) -> bool {
        self.active
            || self.pending.is_some()
            || self
                .attempts
                .iter()
                .any(|a| a.result.is_err() || matches!(a.result, Ok(BindReport::Partial { .. })))
    }
    pub fn invalidate(&self) {
        self.readable.set(false);
        self.editor_area.set(None);
        self.ui.borrow_mut().invalidate();
    }
    pub fn changed(&mut self) {
        self.invalidate();
        self.confirmation = None;
        if self.screen == Screen::Confirm {
            self.screen = Screen::Form;
        }
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
        self.changed();
        task
    }
    fn scan(&mut self, scope: ScanScope) -> Option<Task> {
        if self.pending.as_ref().is_some_and(|t| t.job.writes()) {
            return None;
        }
        self.changed();
        self.observation = None;
        self.scope = scope.clone();
        self.screen = Screen::Candidates;
        if self.pending.is_some() {
            self.requested_scan = Some(scope);
            None
        } else {
            Some(self.issue(Job::Scan(scope)))
        }
    }
    fn close(&mut self) {
        self.active = false;
        self.requested_scan = None;
        self.changed();
        // Pending ownership and all submitted outcomes survive closing.
    }
    fn select(&mut self, key: &PaneKey) {
        if self.pending.as_ref().is_some_and(|t| t.job.writes()) {
            return;
        }
        self.changed();
        self.selected = Some(key.clone());
        self.screen = Screen::Details;
    }
    fn selected_pane(&self) -> Option<&model::ObservedPane> {
        self.observation
            .as_ref()?
            .panes
            .iter()
            .find(|p| Some(&p.key) == self.selected.as_ref())
    }
    fn review(&mut self, selector: AgentSelector, save: bool) -> bool {
        self.changed();
        if self.pending.is_some() {
            return false;
        }
        let valid = match &selector {
            AgentSelector::NewName(name) => {
                !name.trim().is_empty() && !name.chars().any(|c| c < '\u{20}' || c == '\u{7f}')
            }
            AgentSelector::ExistingUuid(id) => model::canonical_uuid(id),
        };
        let Some(pane) = self.selected_pane().filter(|p| p.adoptable) else {
            return false;
        };
        if !valid {
            return false;
        }
        self.confirmation = Some(BindSelection {
            key: pane.key.clone(),
            expect: pane.expect.clone().expect("validated Core expectation"),
            selector,
            save,
        });
        self.screen = Screen::Confirm;
        true
    }
    fn confirm(&mut self) -> Option<Task> {
        if self.screen != Screen::Confirm || !self.readable.get() || self.pending.is_some() {
            return None;
        }
        let selection = self.confirmation.take()?;
        let pane = self.selected_pane()?;
        if selection.key != pane.key
            || pane.expect.as_ref() != Some(&selection.expect)
            || !pane.adoptable
        {
            return None;
        }
        self.screen = Screen::Results;
        Some(self.issue(Job::Bind(selection)))
    }
    pub fn loaded(&mut self, completed: load::Completed) -> Completion {
        let mut next = Completion {
            task: None,
            bound_uuid: None,
        };
        if self.pending.as_ref() != Some(&completed.task) {
            return next;
        }
        self.pending = None;
        self.changed();
        match completed.task.job {
            Job::Scan(_) if !self.active => self.requested_scan = None,
            Job::Scan(_) => {
                if let Some(scope) = self.requested_scan.take() {
                    next.task = Some(self.issue(Job::Scan(scope)));
                    return next;
                }
                match completed.result {
                    Ok(Outcome::Scan(observation)) => {
                        // Preserve a stable pane anchor, never transfer authority
                        // to the row that happens to inherit its old index.
                        if self
                            .selected
                            .as_ref()
                            .is_some_and(|key| !observation.panes.iter().any(|p| p.key == *key))
                        {
                            self.selected = None;
                        }
                        for identity in observation.panes.iter().filter_map(|p| p.identity.as_ref())
                        {
                            if let Some(known) = self.known.iter_mut().find(|k| k.id == identity.id)
                            {
                                *known = identity.clone();
                            } else {
                                self.known.push(identity.clone());
                            }
                        }
                        self.observation = Some(observation);
                        self.screen = Screen::Candidates;
                        self.failure = None;
                    }
                    Err(error) => self.failure = Some(error),
                    _ => self.failure = Some(model::invalid()),
                }
            }
            Job::Bind(selection) => {
                let result = match completed.result {
                    Ok(Outcome::Bind(report)) => Ok(report),
                    Err(error) => Err(error),
                    _ => Err(model::invalid()),
                };
                if let Ok(BindReport::Bound { id, .. }) = &result {
                    next.bound_uuid = Some(id.clone());
                }
                self.attempts.push(Attempt { selection, result });
                self.result_index = self.attempts.len() - 1;
                self.observation = None;
                self.screen = Screen::Results;
                self.requested_scan = None;
            }
        }
        next
    }
    fn choose(&mut self, id: &str) -> Option<Task> {
        if id == "close" {
            self.close();
            return None;
        }
        if self.pending.as_ref().is_some_and(|t| t.job.writes()) {
            return None;
        }
        match id {
            "scan" => return self.scan(self.scope.clone()),
            "back" | "cancel" => {
                self.screen = Screen::Candidates;
                self.changed();
            }
            "bind"
                if self.selected_pane().is_some_and(|p| p.adoptable) && self.pending.is_none() =>
            {
                let id = self
                    .selected_pane()
                    .and_then(|p| p.identity.as_ref())
                    .map(|i| i.id.clone());
                self.existing = id.is_some();
                self.save = false;
                self.form_focus = true;
                self.screen = Screen::Form;
                self.editor = id.unwrap_or_default().into();
                self.ui = RefCell::new(picker_surface::State::new(None, vec![], None));
                self.changed();
            }
            "selector" => {
                self.existing = !self.existing;
                self.editor.clear();
                self.changed();
            }
            "lifetime" => {
                self.save = !self.save;
                self.changed();
            }
            "review" => {
                let query = self.editor.as_str().to_owned();
                let selector = if self.existing {
                    AgentSelector::ExistingUuid(query)
                } else {
                    AgentSelector::NewName(query)
                };
                if !self.review(selector, self.save) {
                    self.failure = Some(SquadError::new(
                        "USAGE_ERROR",
                        "Enter a new name, or choose an existing identity by its lowercase UUID.",
                    ));
                }
            }
            "confirm" => {
                if !self.readable.get() {
                    self.failure = Some(SquadError::new(
                        "USAGE_ERROR",
                        "Enlarge the terminal until the complete target, identity and both Cancel/Confirm choices are visible.",
                    ));
                    return None;
                }
                return self.confirm();
            }
            "results" if !self.attempts.is_empty() => {
                self.screen = Screen::Results;
                self.form_focus = true;
                self.changed();
            }
            "previous-result" => {
                self.result_index = self.result_index.saturating_sub(1);
                self.changed();
            }
            "next-result" => {
                self.result_index =
                    (self.result_index + 1).min(self.attempts.len().saturating_sub(1));
                self.changed();
            }
            id if id.starts_with("candidate:") => {
                if let Some(key) = id
                    .strip_prefix("candidate:")
                    .and_then(|i| i.parse::<usize>().ok())
                    .and_then(|i| self.observation.as_ref()?.panes.get(i))
                    .map(|p| p.key.clone())
                {
                    self.select(&key);
                    self.form_focus = true;
                }
            }
            id if id.starts_with("known:") => {
                if let Some(agent) = self
                    .known
                    .iter()
                    .find(|a| id.strip_prefix("known:") == Some(a.id.as_str()))
                {
                    self.existing = true;
                    self.editor = agent.id.clone().into();
                    self.form_focus = true;
                    self.changed();
                }
            }
            _ => {}
        }
        None
    }
}
