//! Shell routing over the admitted workspace snapshot. This controller does
//! no IO; the existing action lane owns confirmed Config/membership effects.
use super::*;
use crate::board::{
    shell::OpsTab,
    workspace::{
        OpsTarget, OpsTreeRow, SelectionState, manage::Kind, observe::OpsWorkspaceSnapshot,
    },
};
use crate::{
    action::OpsUiAction,
    config::{OpsKeyScope, OpsPreferences},
};
use std::rc::Rc;

/// Rows are shared only within an immutable paint, never across an input or
/// snapshot update. Nested paints and unwinding restore the previous scope.
pub(in crate::board) struct FrameRows<'a> {
    slot: &'a RefCell<Option<Rc<[OpsTreeRow]>>>,
    previous: Option<Rc<[OpsTreeRow]>>,
}

impl Drop for FrameRows<'_> {
    fn drop(&mut self) {
        *self.slot.borrow_mut() = self.previous.take();
    }
}

impl App {
    pub(in crate::board) fn workspace_snapshot(&self) -> Option<&OpsWorkspaceSnapshot> {
        Some(&self.view.as_ref()?.home.as_ref()?.workspace)
    }
    pub(in crate::board) fn workspace_rows(&self) -> Rc<[OpsTreeRow]> {
        if let Some(rows) = self.frame_workspace_rows.borrow().as_ref() {
            return Rc::clone(rows);
        }
        self.workspace_snapshot()
            .map(|snapshot| {
                if self.shell.tab == OpsTab::Digest {
                    crate::board::digest_dashboard::rows(
                        snapshot,
                        &self.workspace.query,
                        self.workspace.selected.as_ref(),
                    )
                } else {
                    self.workspace.rows(&snapshot.roster, &snapshot.preferences)
                }
            })
            .unwrap_or_default()
            .into()
    }
    pub(in crate::board) fn workspace_frame_rows(&self) -> FrameRows<'_> {
        let rows = self.workspace_rows();
        FrameRows {
            previous: self.frame_workspace_rows.replace(Some(rows)),
            slot: &self.frame_workspace_rows,
        }
    }
    pub(in crate::board) fn workspace_selection_state(&self) -> SelectionState {
        let Some(snapshot) = self.workspace_snapshot() else {
            return SelectionState::Unknown;
        };
        self.workspace
            .selection_state(&snapshot.roster, &self.workspace_rows())
    }
    pub(in crate::board) fn workspace_selection_visible(&self) -> bool {
        self.workspace_selection_state() == SelectionState::Visible
    }
    pub(in crate::board) fn workspace_squad(&self) -> Option<&crate::board::workspace::OpsSquad> {
        if self.shell.tab == OpsTab::Overview {
            return None;
        }
        let id = match self.workspace.selected.as_ref()? {
            OpsTarget::Squad(id)
            | OpsTarget::Member {
                room_id: Some(id), ..
            } => id,
            _ => return None,
        };
        self.workspace_snapshot()?
            .roster
            .squads
            .iter()
            .find(|squad| &squad.room_id == id)
    }
    pub(in crate::board) fn workspace_selected_member(&self) -> Option<&OpsTarget> {
        self.workspace.selected.as_ref().filter(|target| {
            self.workspace_selection_visible() && matches!(target, OpsTarget::Member { .. })
        })
    }
    pub(in crate::board) fn workspace_scope(&self) -> OpsKeyScope {
        if self.focused_pane() == Some(Pane::Replies) {
            return OpsKeyScope::Reader;
        }
        if self.shell.tab == OpsTab::Overview {
            return OpsKeyScope::Overview;
        }
        match self.workspace.selected {
            Some(OpsTarget::Squad(_)) => OpsKeyScope::Squad,
            Some(OpsTarget::Member { .. }) => OpsKeyScope::Member,
            _ => OpsKeyScope::Pool,
        }
    }
    pub(in crate::board) fn target_squad(&self, target: &RowTarget) -> Option<String> {
        match target {
            RowTarget::Workspace(OpsTarget::Member {
                room_id: Some(room),
                ..
            }) => self
                .workspace_snapshot()?
                .roster
                .squads
                .iter()
                .find(|squad| squad.room_id == *room)
                .map(|squad| squad.name.clone()),
            RowTarget::Workspace(OpsTarget::Member { room_id: None, .. }) => Some(String::new()),
            RowTarget::Workspace(_) => None,
            RowTarget::Member { squad, .. } | RowTarget::Lead { squad, .. } => Some(squad.clone()),
        }
    }
    /// An explicit message action focuses its selected agent's conversation.
    /// Compact Overview retains its monitor, so the editor opens in Agents;
    /// wide Overview keeps its existing shared conversation placement.
    pub(super) fn workspace_focus_conversation(&mut self) {
        let Some(selected) = self.workspace_selected_member().cloned() else {
            return;
        };
        let target = RowTarget::Workspace(selected);
        if self
            .input
            .as_ref()
            .and_then(|input| input.row_send.as_ref())
            .is_none_or(|send| send.target != target)
        {
            return;
        }
        if self.shell.tab == OpsTab::Overview
            && crate::board::conversation::layout::overview(self.conversation_area())
                .conversation
                .is_none()
        {
            self.shell.tab = OpsTab::Agents;
        }
        self.focus = 1;
        self.invalidate_frame_geometry();
    }
    pub(in crate::board) fn workspace_bindings(&self) -> Bindings {
        let mut keys = self.workspace_effective_bindings();
        if self.shell.tab != OpsTab::Agents || self.workspace_scope() == OpsKeyScope::Reader {
            keys.retain(|_, action| {
                !matches!(
                    action.verb,
                    Verb::Ui(
                        OpsUiAction::WorkspaceSnapshots
                            | OpsUiAction::DiscoverAgents
                            | OpsUiAction::RenameMember
                    )
                )
            });
        }
        if !self.usage_chart_available() {
            keys.retain(|_, action| action.verb != Verb::Ui(OpsUiAction::UsageControls));
        }
        if !self.workspace_selection_visible() || self.workspace_scope() != OpsKeyScope::Member {
            keys.retain(|_, action| action.verb != Verb::Ui(OpsUiAction::RenameMember));
        }
        keys
    }
    fn workspace_effective_bindings(&self) -> Bindings {
        let Some(snapshot) = self.workspace_snapshot() else {
            return crate::config::workspace_defaults(self.workspace_scope());
        };
        let scope = self.workspace_scope();
        if let Some(keys) = self
            .workspace
            .selected
            .as_ref()
            .and_then(|target| snapshot.sections.get(target))
            .and_then(|tables| tables.get(&scope))
        {
            return keys.clone();
        }
        let host = self
            .selected_row()
            .and_then(|row| row["pane"]["location"]["host"].as_str());
        host.and_then(|host| snapshot.hosts.get(host))
            .and_then(|tables| tables.get(&scope))
            .or_else(|| snapshot.keys.get(&scope))
            .cloned()
            .unwrap_or_default()
    }
    pub(in crate::board) fn workspace_reconcile(&mut self) {
        self.workspace.query = self.search.clone();
        if let Some(name) = self.workspace_startup.take() {
            if let Some(squad) = self.workspace_snapshot().and_then(|snapshot| {
                snapshot
                    .roster
                    .squads
                    .iter()
                    .find(|squad| squad.name == name)
            }) {
                self.workspace.selected = Some(OpsTarget::Squad(squad.room_id.clone()));
                self.shell.tab = OpsTab::Agents;
            } else {
                self.notice = Some(format!(
                    "Squad '{name}' is unavailable; choose a visible target explicitly."
                ));
            }
        }
        let rows = self.workspace_rows();
        if self.shell.tab == OpsTab::Digest
            && self
                .workspace
                .selected
                .as_ref()
                .is_some_and(|target| !matches!(target, OpsTarget::Member { .. }))
        {
            self.workspace.selected = rows.first().map(|row| row.target.clone());
        }
        if self.workspace.selected.is_none() {
            self.workspace.selected = rows.first().map(|row| row.target.clone());
        }
        if let Some(index) = rows
            .iter()
            .position(|row| Some(&row.target) == self.workspace.selected.as_ref())
        {
            self.selected = index;
        }
        // Hidden/missing anchors deliberately remain selected. No neighboring
        // row acquires recipient authority merely because its index now fits.
        self.last_click = None;
    }
    pub(super) fn workspace_select(&mut self, index: usize) {
        let rows = self.workspace_rows();
        let Some(row) = rows.get(index.min(rows.len().saturating_sub(1))) else {
            return;
        };
        self.workspace.selected = Some(row.target.clone());
        self.workspace_reconcile();
        self.follow = true;
    }
    fn workspace_save(&mut self, next: OpsPreferences) -> Effect {
        let Some(snapshot) = self.workspace_snapshot() else {
            return Effect::None;
        };
        Effect::Act(Request::WorkspacePreferences {
            expected: snapshot.preferences.clone(),
            next,
        })
    }
    pub(super) fn workspace_action(&mut self, action: OpsUiAction) -> Effect {
        use OpsUiAction::*;
        match action {
            Quit => return Effect::Quit,
            QuotaDetails => return self.open_quota_details(),
            UsageControls => return super::super::overview::chart::host::focus(self),
            Help => {
                self.help_state.borrow_mut().open();
                self.help = true;
                return Effect::None;
            }
            Overview | Agents | DigestDashboard => {
                let digit = match action {
                    Overview => '1',
                    Agents => '2',
                    _ => '3',
                };
                if !self.shell.select_digit(digit, self.workspace_input_owner()) {
                    return Effect::None;
                }
                self.usage_chart.get_mut().release_focus();
                self.workspace_reconcile();
                self.focus = 0;
                self.invalidate_frame_geometry();
                return Effect::None;
            }
            ScrollUp | ScrollDown | ScrollPageUp | ScrollPageDown | ScrollTop | ScrollBottom => {
                if self.focused_pane() == Some(Pane::Replies) {
                    let step = match action {
                        ScrollUp => Step::Lines(-1),
                        ScrollDown => Step::Lines(1),
                        ScrollPageUp => Step::Pages(-1),
                        ScrollPageDown => Step::Pages(1),
                        ScrollTop => Step::Top,
                        ScrollBottom => Step::Bottom,
                        _ => unreachable!(),
                    };
                    self.scroll_pane(Pane::Replies, step);
                }
                return Effect::None;
            }
            _ => {}
        }
        if self.loading() {
            return self.say("Loading the workspace…");
        }
        match action {
            WorkspaceSnapshots => return self.open_recovery(),
            DiscoverAgents => return self.open_discovery(),
            Previous => self.workspace_select(self.selected.saturating_sub(1)),
            Next => self.workspace_select(self.selected.saturating_add(1)),
            First => self.workspace_select(0),
            Last => self.workspace_select(usize::MAX),
            PageUp | PageDown => self.page_rows(if action == PageUp { -1 } else { 1 }),
            Search => self.searching = true,
            Fold | Expand | Inspect => {
                if !self.workspace_selection_visible() {
                    return self.say("The selected target is hidden or unavailable; select a visible target explicitly.");
                }
                let Some(target) = self.workspace.selected.clone() else {
                    return Effect::None;
                };
                if matches!(target, OpsTarget::Member { .. }) {
                    if action == Inspect {
                        if self.shell.tab != OpsTab::Digest {
                            self.shell.tab = OpsTab::Agents;
                        }
                        self.focus = 1;
                    }
                } else {
                    let expanded = match &target {
                        OpsTarget::Pool => self.workspace.pool_expanded,
                        OpsTarget::Squad(id) => self.workspace.expanded.contains(id),
                        _ => false,
                    };
                    if action == Inspect
                        || (action == Expand && !expanded)
                        || (action == Fold && expanded)
                    {
                        self.workspace.toggle_expanded(&target);
                    }
                    self.workspace_reconcile();
                }
            }
            Sort | ReverseSort | Pin => {
                let Some(snapshot) = self.workspace_snapshot() else {
                    return Effect::None;
                };
                let mut next = snapshot.preferences.clone();
                match action {
                    Sort => next.sort = next.sort.next(),
                    ReverseSort => next.reversed = !next.reversed,
                    _ if !self.workspace_selection_visible()
                        || !self.workspace.toggle_pin(&mut next) =>
                    {
                        return self.say("Select a visible squad to pin.");
                    }
                    _ => {}
                }
                return self.workspace_save(next);
            }
            Timers => {
                if !self.workspace_selection_visible() {
                    return self.say("Select a visible timer scope explicitly.");
                }
                let scope = self
                    .workspace
                    .selected
                    .as_ref()
                    .and_then(|target| match target {
                        OpsTarget::Squad(id)
                        | OpsTarget::Member {
                            room_id: Some(id), ..
                        } => self
                            .workspace_snapshot()?
                            .roster
                            .squads
                            .iter()
                            .find(|squad| squad.room_id == *id)
                            .map(|squad| squad.room_id.clone()),
                        _ => None,
                    });
                if !self.cron_shown() {
                    return self.say("Timers have not been read yet.");
                }
                self.cron_list = Some(super::super::cronboard::List::open_scoped(
                    &self.cron, scope, None,
                ));
                return Effect::None;
            }
            RenameMember => return self.workspace_begin(Kind::RenameMember),
            Manage | CreateSquad | RemoveMembership | MoveMembership => {
                return self.workspace_management(action);
            }
            Overview | Agents | DigestDashboard | QuotaDetails | UsageControls | Help | Quit
            | ScrollUp | ScrollDown | ScrollPageUp | ScrollPageDown | ScrollTop | ScrollBottom => {
                unreachable!("shell actions handled first")
            }
        }
        Effect::None
    }
    fn workspace_input_owner(&self) -> crate::board::shell::OpsInputOwner {
        use crate::board::shell::OpsInputOwner;
        if self.mentions.completion.open() {
            OpsInputOwner::Completion
        } else if self.modal_open() {
            OpsInputOwner::Popup
        } else if self.input.is_some() {
            OpsInputOwner::Composer
        } else if self.searching {
            OpsInputOwner::Search
        } else if self.focused_pane() == Some(Pane::Replies) {
            OpsInputOwner::Reader
        } else {
            OpsInputOwner::List
        }
    }
    pub(super) fn workspace_key(&mut self, key: KeyEvent) -> Effect {
        self.restore_focus();
        if self.workspace_form.as_ref().is_some_and(|draft| {
            draft.shown
                && matches!(
                    draft.stage,
                    crate::board::workspace::manage::Stage::Pending
                        | crate::board::workspace::manage::Stage::Result(_)
                )
        }) {
            if key.code == KeyCode::Esc {
                self.workspace_close();
            }
            return Effect::None;
        }
        if self.focused_pane() == Some(Pane::Replies) {
            if key.code == KeyCode::Esc {
                self.focus = 0;
                return Effect::None;
            }
            if matches!(key.code, KeyCode::Char('1' | '2' | '3')) {
                return Effect::None;
            }
        }
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            return self.say("Squad tab reorder is retired; pin or sort squads in the workspace.");
        }
        if let Some(action) = event_name(key).and_then(|name| self.bindings().remove(&name)) {
            return self.perform(&action);
        }
        if key.code == KeyCode::Esc {
            return Effect::Quit;
        }
        Effect::None
    }

    fn workspace_management(&mut self, action: OpsUiAction) -> Effect {
        use crate::board::workspace::manage::Kind;
        match action {
            OpsUiAction::CreateSquad => self.workspace_begin(Kind::Create),
            OpsUiAction::RemoveMembership => self.workspace_begin(Kind::Remove),
            OpsUiAction::MoveMembership => self.workspace_begin(Kind::Assign),
            OpsUiAction::Manage => {
                if self.workspace_selected_member().is_some() {
                    return self.workspace_begin(Kind::Assign);
                }
                if !self.workspace_selection_visible() {
                    return self.say("Select a visible squad explicitly.");
                }
                let title = self
                    .selected_row()
                    .and_then(|row| row["name"].as_str())
                    .unwrap_or("Workspace")
                    .to_owned();
                let mut entries = Vec::new();
                if matches!(self.workspace.selected, Some(OpsTarget::Squad(_))) {
                    for (label, kind) in [
                        ("Add member", Kind::Add),
                        ("Set/clear lead", Kind::Lead),
                        ("Rename squad", Kind::RenameSquad),
                    ] {
                        entries.push(MenuEntry {
                            key: String::new(),
                            label: label.into(),
                            choice: Choice::WorkspaceBegin(kind),
                        });
                    }
                    for (label, action) in [
                        ("Pin/unpin", OpsUiAction::Pin),
                        ("Timers", OpsUiAction::Timers),
                    ] {
                        entries.push(MenuEntry {
                            key: String::new(),
                            label: label.into(),
                            choice: Choice::Action(Action::parse(action.spec().name).unwrap()),
                        });
                    }
                }
                entries.push(MenuEntry {
                    key: String::new(),
                    label: "Create squad".into(),
                    choice: Choice::WorkspaceBegin(Kind::Create),
                });
                if let Some(action) = self
                    .workspace_bindings()
                    .values()
                    .find(|action| {
                        matches!(
                            action.verb,
                            crate::action::Verb::Ui(OpsUiAction::DiscoverAgents)
                        )
                    })
                    .cloned()
                {
                    entries.push(MenuEntry {
                        key: String::new(),
                        label: "Add an agent already running".into(),
                        choice: Choice::Action(action),
                    });
                }
                self.menu = Some(Menu {
                    row_send: None,
                    link: None,
                    prefill: String::new(),
                    title,
                    entries,
                    selected: 0,
                    surface: Default::default(),
                });
                Effect::None
            }
            _ => unreachable!("management action vocabulary"),
        }
    }

    pub(in crate::board) fn workspace_settings(&mut self) -> Effect {
        use crate::config::OpsLayout;
        let Some(snapshot) = self.workspace_snapshot() else {
            return self.say("The workspace has not loaded.");
        };
        let current = snapshot.preferences.layout;
        let mut entries: Vec<_> = [
            (OpsLayout::Auto, "Auto layout"),
            (OpsLayout::Side, "Side layout"),
            (OpsLayout::Stack, "Stack layout"),
        ]
        .into_iter()
        .map(|(layout, label)| MenuEntry {
            key: String::new(),
            label: format!(
                "{label}{}",
                if current == layout { " (current)" } else { "" }
            ),
            choice: Choice::WorkspaceLayout(layout),
        })
        .collect();
        for (label, action) in [
            ("Theme", "theme"),
            ("Token window", "token-window"),
            ("Actions", "menu"),
        ] {
            entries.push(MenuEntry {
                key: String::new(),
                label: label.into(),
                choice: Choice::Action(Action::parse(action).expect("workspace setting action")),
            });
        }
        self.menu = Some(Menu {
            row_send: None,
            link: None,
            prefill: String::new(),
            title: "Workspace settings · minimum usable size governs placement".into(),
            entries,
            selected: 0,
            surface: Default::default(),
        });
        Effect::None
    }
    pub(in crate::board) fn workspace_layout(
        &mut self,
        layout: crate::config::OpsLayout,
    ) -> Effect {
        let Some(snapshot) = self.workspace_snapshot() else {
            return self.say("The workspace has not loaded.");
        };
        let expected = snapshot.preferences.clone();
        let mut next = expected.clone();
        next.layout = layout;
        Effect::Act(Request::WorkspacePreferences { expected, next })
    }

    pub(in crate::board) fn finished_workspace(
        &mut self,
        outcome: Result<crate::membership::Outcome, crate::core::SquadError>,
    ) {
        let message = match outcome {
            Ok(outcome) if outcome.complete => format!(
                "Applied. The workspace will refresh.\n{}",
                serde_json::to_string_pretty(&outcome.document).unwrap_or_default()
            ),
            Ok(outcome) => {
                let repair = outcome.document["repair"]
                    .as_str()
                    .map(|repair| format!(" Repair explicitly: {repair}"))
                    .unwrap_or_default();
                let error=outcome.document["failed"]["error"]["message"].as_str().unwrap_or("Some requested steps were not confirmed; inspect the refreshed membership before another operation.");
                format!(
                    "Partially applied or unknown: {error}.{repair} Nothing is retried automatically.\n{}",
                    operation_results(&outcome.document)
                )
            }
            Err(error) => {
                let status = if matches!(
                    error.code.as_str(),
                    "ROOM_REVISION_CONFLICT"
                        | "SQUAD_ALREADY_EXISTS"
                        | "SQUAD_NAME_INVALID"
                        | "SQUAD_IDENTITY_NOT_SAVED"
                        | "IDENTITY_NAME_CONFLICT"
                ) {
                    "Not applied"
                } else {
                    "Unknown"
                };
                format!(
                    "{status}: {}. Close and refresh before another operation; nothing is retried automatically.",
                    error.message
                )
            }
        };
        self.notice = Some(message.clone());
        if let Some(draft) = self
            .workspace_form
            .as_mut()
            .filter(|draft| draft.kind != Kind::RenameMember)
        {
            draft.stage = crate::board::workspace::manage::Stage::Result(message);
            draft.invalidate();
            *draft.scroll.borrow_mut() = Default::default();
        }
    }
}

/// Preserve per-item evidence rather than replacing a partial batch with a
/// generic success/failure banner. Errors can follow a write, so say Unknown.
fn operation_results(document: &Value) -> String {
    fn collect(value: &Value, lines: &mut Vec<String>) {
        if let Some(results) = value.get("results").and_then(Value::as_array) {
            for item in results {
                let name = item["name"]
                    .as_str()
                    .or_else(|| item["id"].as_str())
                    .unwrap_or("item");
                if let Some(error) = item.get("error") {
                    lines.push(format!(
                        "Unknown or not applied: {name} · {}: {}",
                        error["code"].as_str().unwrap_or("error"),
                        error["message"].as_str().unwrap_or("not confirmed")
                    ));
                } else {
                    lines.push(format!("Applied: {name}"));
                }
            }
        }
        if let Some(object) = value.as_object() {
            for (key, child) in object {
                if key != "results" {
                    collect(child, lines);
                }
            }
        }
    }
    let mut lines = Vec::new();
    collect(document, &mut lines);
    if lines.is_empty() {
        serde_json::to_string_pretty(document).unwrap_or_default()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
pub(in crate::board) mod tests;
