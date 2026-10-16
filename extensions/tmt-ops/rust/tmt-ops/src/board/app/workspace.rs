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
    pub(in crate::board) fn workspace_order(&self) -> crate::board::view_state::Order {
        self.workspace.order.unwrap_or_else(|| {
            self.workspace_snapshot()
                .map(|snapshot| crate::board::view_state::Order::from(&snapshot.preferences))
                .unwrap_or_default()
        })
    }
    fn changed_view_order(&mut self, order: crate::board::view_state::Order) -> Effect {
        self.workspace.order = Some(order);
        self.overview_rows.borrow_mut().clear();
        self.workspace_reconcile();
        self.invalidate_frame_geometry();
        Effect::None
    }
    pub(in crate::board) fn workspace_snapshot(&self) -> Option<&OpsWorkspaceSnapshot> {
        Some(&self.view.as_ref()?.home.as_ref()?.workspace)
    }
    pub(in crate::board) fn digest_window(&self) -> crate::config::DigestWindow {
        self.workspace_snapshot()
            .map_or(crate::config::DigestWindow::default(), |snapshot| {
                snapshot.preferences.digest_window
            })
    }
    pub(in crate::board) fn select_digest_window(
        &mut self,
        window: crate::config::DigestWindow,
    ) -> Effect {
        let Some(snapshot) = self.workspace_snapshot() else {
            return self.say("The workspace has not loaded.");
        };
        let mut next = snapshot.preferences.clone();
        next.digest_window = window;
        self.workspace_save(next)
    }
    /// The unit on screen: the saved choice, except where its window cannot
    /// show it (lifetime has no percent).
    pub(in crate::board) fn savings_unit(&self) -> crate::config::SavingsUnit {
        self.workspace_snapshot()
            .map_or(crate::config::SavingsUnit::default(), |snapshot| {
                snapshot.preferences.savings_unit
            })
            .shown(self.digest_window())
    }
    /// One click or key on any savings figure moves every surface to the next
    /// unit; the choice is the shared preference, never a view-local state.
    pub(in crate::board) fn cycle_savings_unit(&mut self) -> Effect {
        let Some(snapshot) = self.workspace_snapshot() else {
            return self.say("The workspace has not loaded.");
        };
        let mut next = snapshot.preferences.clone();
        next.savings_unit = self.savings_unit().next(self.digest_window());
        self.workspace_save(next)
    }
    pub(in crate::board) fn digest_savings<'a>(
        &self,
        identities: impl IntoIterator<Item = &'a str>,
    ) -> crate::labels::digest::SavingsSummary {
        crate::labels::digest::summarize_savings(
            self.labels.digest(),
            identities,
            self.digest_window(),
        )
    }
    /// Whether this frame painted an estimate that answers the unit key.
    pub(in crate::board) fn savings_visible(&self) -> bool {
        self.workspace_strip_hits
            .borrow()
            .iter()
            .any(|(_, choice)| *choice == crate::board::app::Choice::SavingsUnit)
    }
    /// A painted estimate answers a click with the next unit. The rectangle is
    /// the caption as painted and clipped, so a hidden figure has no hit.
    pub(in crate::board) fn savings_hit(&self, x: u16, y: u16, width: usize, clip: u16) {
        let width = width.min(usize::from(clip)) as u16;
        if width > 0 {
            self.workspace_strip_hits.borrow_mut().push((
                ratatui::layout::Rect::new(x, y, width, 1),
                crate::board::app::Choice::SavingsUnit,
            ));
        }
    }
    /// The caption in the unit on screen, through the one owner of its wording.
    pub(in crate::board) fn savings_caption(
        &self,
        summary: &crate::labels::digest::SavingsSummary,
        width: usize,
    ) -> String {
        crate::board::digest_dashboard::savings_caption(
            summary,
            self.digest_window(),
            self.savings_unit(),
            width,
        )
    }
    pub(in crate::board) fn digest_savings_start(
        &self,
        summary: &crate::labels::digest::SavingsSummary,
        width: usize,
    ) -> Option<String> {
        crate::board::digest_dashboard::savings_start(
            summary,
            self.digest_window(),
            self.local_zone.as_ref().unwrap_or(&jiff::tz::TimeZone::UTC),
            width,
        )
    }
    pub(in crate::board) fn workspace_rows(&self) -> Rc<[OpsTreeRow]> {
        if let Some(rows) = self.frame_workspace_rows.borrow().as_ref() {
            return Rc::clone(rows);
        }
        if self.shell.tab == OpsTab::Overview
            && let Some(snapshot) = self.workspace_snapshot()
        {
            return crate::board::overview::monitor::rows(self, snapshot);
        }
        self.workspace_snapshot()
            .map(|snapshot| {
                if self.shell.tab == OpsTab::Digest {
                    crate::board::workspace::identity_rows(
                        snapshot,
                        &self.workspace.query,
                        self.workspace.selected.as_ref(),
                        &crate::board::workspace::OpsVisibility {
                            scope: self.workspace.visibility.scope.clone(),
                            reveal_hidden: true,
                        },
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
        let target_shown = match self.shell.tab {
            OpsTab::Overview => self
                .overview_panels()
                .shown(crate::config::OverviewPanel::Monitor),
            OpsTab::Digest => self.digest_panels().shown(crate::config::DigestPanel::List),
            _ => true,
        };
        target_shown && self.workspace_selection_state() == SelectionState::Visible
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
    pub(in crate::board) fn select_overview_view(
        &mut self,
        view: crate::board::overview::monitor::View,
    ) -> Effect {
        if self.shell.tab != OpsTab::Overview || self.overview_view == view {
            return Effect::None;
        }
        self.overview_view = view;
        self.overview_rows.borrow_mut().clear();
        self.workspace.selected = self.workspace_rows().first().map(|row| row.target.clone());
        self.scrolls
            .scroll(Pane::Rows, crate::board::scroll::Step::Top);
        self.workspace_reconcile();
        self.invalidate_frame_geometry();
        Effect::None
    }
    /// A squad selection grants a conversation only to its observed, current lead.
    pub(in crate::board) fn workspace_message_target(&self) -> Option<OpsTarget> {
        if self.shell.tab == OpsTab::Overview
            && self.workspace_selection_visible()
            && let Some(OpsTarget::Squad(room)) = self.workspace.selected.as_ref()
        {
            let snapshot = self.workspace_snapshot()?;
            let squad = snapshot.roster.squads.iter().find(|s| &s.room_id == room)?;
            let lead = squad.lead.as_ref()?;
            if !squad.members.contains(lead) {
                return None;
            }
            let target = OpsTarget::Member {
                room_id: Some(room.clone()),
                identity_id: lead.clone(),
            };
            return (snapshot.roster.contains(&target)
                && snapshot
                    .rows
                    .get(&target)
                    .is_some_and(|row| row["id"].as_str() == Some(lead.as_str())))
            .then_some(target);
        }
        self.workspace_selected_member().cloned()
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
        let Some(selected) = self.workspace_message_target() else {
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
        // Only this explicit, validated message path establishes a temporary
        // reveal. Tab and pointer eligibility are checked before focus changes.
        self.focus = 1;
        if self.shell.tab == OpsTab::Overview
            && self
                .overview_placement(self.conversation_area())
                .conversation
                .is_none()
        {
            self.shell.tab = OpsTab::Agents;
        }
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
                            | OpsUiAction::AgentsFilter
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
        let squad = self
            .workspace
            .selected
            .as_ref()
            .and_then(|target| {
                if let OpsTarget::Squad(id) = target {
                    Some(id)
                } else {
                    None
                }
            })
            .filter(|_| self.workspace_selection_visible());
        keys.retain(|_, action| match action.verb {
            Verb::Ui(OpsUiAction::HideSquad) => {
                self.shell.tab == OpsTab::Agents
                    && squad.is_some_and(|id| {
                        self.workspace_snapshot()
                            .is_some_and(|snapshot| !snapshot.preferences.hidden.contains(id))
                    })
            }
            Verb::Ui(OpsUiAction::ShowSquad) => {
                self.shell.tab == OpsTab::Agents
                    && squad.is_some_and(|id| {
                        self.workspace_snapshot()
                            .is_some_and(|snapshot| snapshot.preferences.hidden.contains(id))
                    })
            }
            Verb::Ui(OpsUiAction::RevealHidden) => {
                self.workspace.visibility.scope.is_empty()
                    && !self.workspace.visibility.reveal_hidden
                    && self
                        .workspace_snapshot()
                        .is_some_and(|snapshot| snapshot.hidden_count > 0)
            }
            Verb::Ui(OpsUiAction::ClearScope) => !self.workspace.visibility.scope.is_empty(),
            _ => true,
        });
        keys
    }
    pub(in crate::board) fn workspace_binding_sources(
        &self,
    ) -> Option<&std::collections::BTreeMap<String, String>> {
        let snapshot = self.workspace_snapshot()?;
        let scope = self.workspace_scope();
        let section = self.workspace.selected.as_ref().filter(|target| {
            snapshot
                .sections
                .get(*target)
                .is_some_and(|tables| tables.contains_key(&scope))
        });
        let host = self
            .selected_row()
            .and_then(|row| row["pane"]["location"]["host"].as_str())
            .filter(|host| {
                snapshot
                    .hosts
                    .get(*host)
                    .is_some_and(|tables| tables.contains_key(&scope))
            });
        let key = if let Some(target) = section {
            (
                scope,
                crate::board::workspace::observe::BindingContext::Section(target.clone()),
            )
        } else {
            (
                scope,
                host.map_or(
                    crate::board::workspace::observe::BindingContext::Global,
                    |host| crate::board::workspace::observe::BindingContext::Host(host.to_owned()),
                ),
            )
        };
        snapshot.binding_sources.get(&key)
    }
    fn workspace_effective_bindings(&self) -> Bindings {
        let host = self
            .selected_row()
            .and_then(|row| row["pane"]["location"]["host"].as_str());
        self.workspace_context_bindings(
            self.workspace.selected.as_ref(),
            self.workspace_scope(),
            host,
        )
    }
    /// A displayed mention has an explicit member context; ordinary bindings
    /// use the selected context through the same section/host/global owner.
    pub(in crate::board) fn workspace_context_bindings(
        &self,
        target: Option<&OpsTarget>,
        scope: OpsKeyScope,
        host: Option<&str>,
    ) -> Bindings {
        let Some(snapshot) = self.workspace_snapshot() else {
            return crate::config::workspace_defaults(scope);
        };
        if let Some(keys) = target
            .and_then(|target| snapshot.sections.get(target))
            .and_then(|tables| tables.get(&scope))
        {
            return keys.clone();
        }
        host.and_then(|host| snapshot.hosts.get(host))
            .and_then(|tables| tables.get(&scope))
            .or_else(|| snapshot.keys.get(&scope))
            .cloned()
            .unwrap_or_default()
    }
    pub(in crate::board) fn workspace_reconcile(&mut self) {
        self.workspace.query = self.search.clone();
        if let Some(id) = self.workspace_startup.take() {
            if let Some(squad) = self.workspace_snapshot().and_then(|snapshot| {
                snapshot
                    .roster
                    .squads
                    .iter()
                    .find(|squad| squad.room_id == id)
            }) {
                self.workspace.selected = Some(OpsTarget::Squad(squad.room_id.clone()));
                self.shell.tab = OpsTab::Agents;
            } else {
                self.notice = Some(format!(
                    "Scoped squad '{id}' is unavailable; choose a visible target explicitly."
                ));
            }
        }
        let rows = self.workspace_rows();
        if self.shell.tab == OpsTab::Overview
            && let Some(OpsTarget::Member { identity_id, .. }) = self.workspace.selected.as_ref()
            && self.workspace_snapshot().is_some_and(|snapshot|
                self.workspace.selected.as_ref().is_some_and(|target| snapshot.roster.contains(target)))
            && let Some(row) = rows.iter().find(|row|
                matches!(&row.target, OpsTarget::Member { identity_id: id, .. } if id == identity_id))
        {
            // Overview has one stable occurrence per identity. Entering it from
            // another visible membership keeps the UUID; vanished targets stay
            // missing and never acquire a replacement membership automatically.
            self.workspace.selected = Some(row.target.clone());
        }
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
        self.invalidate_frame_geometry();
    }
    fn workspace_save(&mut self, next: OpsPreferences) -> Effect {
        let Some(snapshot) = self.workspace_snapshot() else {
            return Effect::None;
        };
        Effect::Act(Request::WorkspacePreferences {
            expected: snapshot.preferences.clone(),
            next,
            saved: None,
        })
    }
    pub(in crate::board) fn finished_workspace_preferences(
        &mut self,
        preferences: OpsPreferences,
        message: String,
    ) {
        // Only a confirmed explicit Hide of the current visible squad moves
        // selection. Other saves retain missing/filter-hidden UUID anchors.
        let move_from = self.workspace.selected.as_ref().and_then(|target| {
            let OpsTarget::Squad(id) = target else {
                return None;
            };
            let snapshot = self.workspace_snapshot()?;
            (!snapshot.preferences.hidden.contains(id)
                && preferences.hidden.contains(id)
                && !self.workspace.visibility.shows_squad(id, &preferences))
            .then(|| {
                self.workspace_rows()
                    .iter()
                    .position(|row| &row.target == target)
            })
            .flatten()
        });
        if let Some(snapshot) = self.view.as_mut().and_then(|view| view.home.as_mut()) {
            snapshot.workspace.set_preferences(preferences);
        }
        self.overview_rows.borrow_mut().clear();
        if let Some(index) = move_from {
            let rows = self.workspace_rows();
            self.workspace.selected = rows
                .get(index.min(rows.len().saturating_sub(1)))
                .map(|row| row.target.clone());
        }
        self.workspace_reconcile();
        self.invalidate_frame_geometry();
        self.finished(Ok(message));
    }

    pub(in crate::board) fn workspace_owner_timers(
        &mut self,
        room: Option<String>,
        owner: Option<String>,
    ) -> Effect {
        if !self.cron_shown() {
            return self.say("Timers have not been read yet.");
        }
        let Some(snapshot) = self.workspace_snapshot() else {
            return Effect::None;
        };
        if room
            .as_ref()
            .is_some_and(|id| !snapshot.roster.squads.iter().any(|s| &s.room_id == id))
            || owner
                .as_ref()
                .is_some_and(|id| !snapshot.roster.members.contains_key(id))
        {
            return self.say("The timer scope is no longer available; select it again.");
        }
        self.cron_list = Some(super::super::cronboard::List::open_owner(
            &self.cron, room, owner, None,
        ));
        Effect::None
    }

    pub(super) fn workspace_action(&mut self, action: OpsUiAction) -> Effect {
        use OpsUiAction::*;
        match action {
            Quit => return Effect::Quit,
            QuotaDetails => return self.open_quota_details(),
            SavingsUnit => return self.cycle_savings_unit(),
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
                self.conversation_return = None;
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
            ClearScope => {
                self.workspace.visibility.scope.clear();
                self.workspace_startup = None;
                self.overview_rows.borrow_mut().clear();
                self.workspace_reconcile();
                self.invalidate_frame_geometry();
            }
            RevealHidden => {
                if !self.workspace.visibility.scope.is_empty() {
                    return Effect::None;
                }
                self.workspace.visibility.reveal_hidden = true;
                self.overview_rows.borrow_mut().clear();
                self.workspace_reconcile();
                self.invalidate_frame_geometry();
            }
            HideSquad | ShowSquad => {
                if self.shell.tab != OpsTab::Agents || !self.workspace_selection_visible() {
                    return self.say("Select a visible squad explicitly.");
                }
                let Some(OpsTarget::Squad(id)) = self.workspace.selected.as_ref() else {
                    return self.say("Common pool cannot be hidden. Select a squad explicitly.");
                };
                let Some(snapshot) = self.workspace_snapshot() else {
                    return Effect::None;
                };
                let mut next = snapshot.preferences.clone();
                if action == HideSquad {
                    if next.hidden.len() >= 1024 && !next.hidden.contains(id) {
                        return self.say("Hidden squad limit reached (1024); show a squad first.");
                    }
                    next.hidden.insert(id.clone());
                } else {
                    next.hidden.remove(id);
                }
                return self.workspace_save(next);
            }
            AgentsFilter if self.shell.tab == OpsTab::Agents => {
                return self.agents_filter(self.workspace.filter.next());
            }
            AgentsFilter => return Effect::None,
            OverviewView if self.shell.tab == OpsTab::Overview => {
                return self.select_overview_view(self.overview_view.next());
            }
            OverviewView => return Effect::None,
            MonitorDetails => return self.overview_details(),
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
                            self.conversation_return.get_or_insert(self.shell.tab);
                            self.shell.tab = OpsTab::Agents;
                            if let OpsTarget::Member {
                                room_id: Some(room),
                                ..
                            } = &target
                            {
                                self.workspace.expanded.insert(room.clone());
                            } else {
                                self.workspace.pool_expanded = true;
                            }
                            self.workspace_reconcile();
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
            Sort | ReverseSort => {
                let mut order = self.workspace_order();
                match (action, self.shell.tab) {
                    (Sort, OpsTab::Overview) => {
                        order.overview = Some(order.overview.map_or_else(
                            || {
                                crate::config::OverviewSort::initial(
                                    crate::config::OverviewColumn::Agent,
                                )
                            },
                            |value| value.next(),
                        ))
                    }
                    (ReverseSort, OpsTab::Overview) => {
                        order.overview = Some(
                            order
                                .overview
                                .unwrap_or_else(|| {
                                    crate::config::OverviewSort::initial(
                                        crate::config::OverviewColumn::Agent,
                                    )
                                })
                                .reverse(),
                        )
                    }
                    (Sort, _) => order.agents = order.agents.next(),
                    _ => order.reversed = !order.reversed,
                }
                return self.changed_view_order(order);
            }
            Pin => {
                let Some(snapshot) = self.workspace_snapshot() else {
                    return Effect::None;
                };
                let mut next = snapshot.preferences.clone();
                match action {
                    Pin if self.shell.tab == OpsTab::Overview => {
                        if !self.workspace_selection_visible() {
                            return self.say("Select a visible agent to pin.");
                        }
                        let Some(OpsTarget::Member { identity_id, .. }) =
                            self.workspace.selected.as_ref()
                        else {
                            return self.say("Select a visible agent to pin.");
                        };
                        let unpinned = next.agent_pins.remove(identity_id);
                        if !unpinned {
                            if next.agent_pins.len() >= 1024 {
                                return self
                                    .say("Agent pin limit reached (1024); unpin an agent first.");
                            }
                            next.agent_pins.insert(identity_id.clone());
                        }
                        let row = self
                            .workspace_rows()
                            .iter()
                            .find(|row| Some(&row.target) == self.workspace.selected.as_ref())
                            .map(|row| row.title.clone())
                            .expect("visible pin target");
                        let name: String = crate::board::notes::sanitize(&row)
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(200)
                            .collect();
                        return Effect::Act(Request::WorkspacePreferences {
                            expected: snapshot.preferences.clone(),
                            next,
                            saved: Some(format!(
                                "{} {name}",
                                if unpinned { "Unpinned" } else { "Pinned" }
                            )),
                        });
                    }
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
                if self.shell.tab == OpsTab::Overview && self.workspace.selected.is_none() {
                    return self.workspace_owner_timers(None, None);
                }
                if !self.workspace_selection_visible() {
                    return self.say("Select a visible timer scope explicitly.");
                }
                let (room, owner) = match self.workspace.selected.as_ref() {
                    Some(OpsTarget::Squad(room)) => (Some(room.clone()), None),
                    Some(OpsTarget::Member {
                        room_id,
                        identity_id,
                    }) => (
                        if self.shell.tab == OpsTab::Overview {
                            None
                        } else {
                            room_id.clone()
                        },
                        Some(identity_id.clone()),
                    ),
                    _ => (None, None),
                };
                return self.workspace_owner_timers(room, owner);
            }
            RenameMember => return self.workspace_begin(Kind::RenameMember),
            Manage | CreateSquad | RemoveMembership | MoveMembership => {
                return self.workspace_management(action);
            }
            Overview | Agents | DigestDashboard | QuotaDetails | SavingsUnit | UsageControls
            | Help | Quit | ScrollUp | ScrollDown | ScrollPageUp | ScrollPageDown | ScrollTop
            | ScrollBottom => {
                unreachable!("shell actions handled first")
            }
        }
        Effect::None
    }

    pub(in crate::board) fn agents_filter(
        &mut self,
        filter: crate::board::workspace::AgentsFilter,
    ) -> Effect {
        if self.shell.tab != OpsTab::Agents || self.workspace_scope() == OpsKeyScope::Reader {
            return Effect::None;
        }
        self.workspace.filter = filter;
        self.workspace_reconcile();
        self.invalidate_frame_geometry();
        self.follow = true;
        Effect::None
    }
    pub(super) fn workspace_input_owner(&self) -> crate::board::shell::OpsInputOwner {
        use crate::board::shell::OpsInputOwner;
        let overlay = self.overlay();
        let editing = match overlay {
            Some(Overlay::Workspace) => self.input.is_some(),
            Some(Overlay::Settings) => self
                .settings
                .as_ref()
                .is_some_and(|overlay| overlay.editing()),
            Some(Overlay::Checklist) => self
                .checklist
                .as_ref()
                .is_some_and(|controller| controller.owns_typing()),
            Some(Overlay::Recovery) => self
                .recovery
                .as_ref()
                .is_some_and(|controller| controller.owns_typing()),
            Some(Overlay::Discovery) => self
                .discovery
                .as_ref()
                .is_some_and(|controller| controller.owns_typing()),
            Some(_) => false,
            None => self.menu.is_none() && self.input.is_some(),
        };
        if editing && self.mentions.completion.open() {
            OpsInputOwner::Completion
        } else if editing {
            OpsInputOwner::Composer
        } else if overlay.is_some() || self.menu.is_some() {
            OpsInputOwner::Popup
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
        if self.focused_pane() == Some(Pane::Replies) && key.code == KeyCode::Esc {
            self.focus = 0;
            if let Some(tab) = self.conversation_return.take() {
                self.shell.tab = tab;
                self.workspace_reconcile();
            }
            self.invalidate_frame_geometry();
            return Effect::None;
        }
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            return self.say("Squad tab reorder is retired; pin or sort squads in the workspace.");
        }
        if let Some(action) = event_name(key).and_then(|name| self.bindings().remove(&name)) {
            return self.perform(&action);
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
                    let action = if self.workspace_squad().is_some_and(|squad| {
                        self.workspace_snapshot().is_some_and(|snapshot| {
                            snapshot.preferences.hidden.contains(&squad.room_id)
                        })
                    }) {
                        OpsUiAction::ShowSquad
                    } else {
                        OpsUiAction::HideSquad
                    };
                    entries.push(MenuEntry {
                        key: String::new(),
                        label: crate::board::view::footer::hint_word(
                            self,
                            &Action::parse(action.spec().name).unwrap(),
                        )
                        .into(),
                        choice: Choice::Action(Action::parse(action.spec().name).unwrap()),
                    });
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
                    anchor: None,
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
        for window in [
            crate::config::DigestWindow::Day,
            crate::config::DigestWindow::Week,
            crate::config::DigestWindow::AllTime,
        ] {
            entries.push(MenuEntry {
                key: String::new(),
                label: format!(
                    "Digest estimate window: {}{}",
                    window.label(),
                    if snapshot.preferences.digest_window == window {
                        " (current)"
                    } else {
                        ""
                    }
                ),
                choice: Choice::DigestWindow(window),
            });
        }
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
        if matches!(self.shell.tab, OpsTab::Overview | OpsTab::Digest) {
            entries.push(MenuEntry {
                key: String::new(),
                label: "Panels".into(),
                choice: Choice::Panels,
            });
        }
        self.menu = Some(Menu {
            row_send: None,
            link: None,
            prefill: String::new(),
            title: "Workspace settings · minimum usable size governs placement".into(),
            entries,
            selected: 0,
            anchor: None,
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
        Effect::Act(Request::WorkspacePreferences {
            expected,
            next,
            saved: None,
        })
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
