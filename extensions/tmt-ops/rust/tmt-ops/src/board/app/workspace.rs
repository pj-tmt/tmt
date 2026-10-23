//! Shell routing over the admitted workspace snapshot. This controller does
//! no IO; the existing action lane owns confirmed Config/membership effects.
use super::*;
use crate::board::{
    shell::OpsTab,
    workspace::{
        AgentsFilter, OpsTarget, OpsTreeRow, SelectionState, manage::Kind,
        observe::OpsWorkspaceSnapshot,
    },
};
use crate::{
    action::OpsUiAction,
    config::{OpsKeyScope, OpsPreferences},
};
use std::rc::Rc;

/// One frame-local view of the destination-owned row projection. The slot never
/// crosses input or snapshot updates; nested scopes and unwinding restore it.
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
    /// A click on a matrix cell selects the row's agent and opens Talk with it. No pair
    /// view exists, so the notice says whose conversation opened.
    pub(in crate::board) fn open_exchange_pair(&mut self, sender: &str, recipient: &str) -> Effect {
        let Some(snapshot) = self.workspace_snapshot() else {
            return Effect::None;
        };
        let name = |id: &str| {
            snapshot
                .roster
                .members
                .get(id)
                .map_or_else(|| id.to_owned(), |member| member.name.clone())
        };
        let notice = format!(
            "Talk opens with {}; there is no pair view for {} and {}.",
            name(sender),
            name(sender),
            name(recipient)
        );
        let current = self.workspace.selected.clone().filter(|target| {
            matches!(target, OpsTarget::Member { identity_id, .. } if identity_id == sender)
        });
        let Some(target) = current.or_else(|| {
            snapshot
                .rows
                .keys()
                .find(|target| {
                    matches!(target, OpsTarget::Member { identity_id, .. } if identity_id == sender)
                })
                .cloned()
        }) else {
            return self.say("The row's agent is no longer observed.");
        };
        self.workspace.selected = Some(target);
        self.workspace_action(OpsUiAction::Inspect);
        self.say(&notice)
    }
    /// A click on an Agents column header: only the order changes.
    pub(in crate::board) fn sort_by_header(
        &mut self,
        header: crate::board::header_sort::Header,
    ) -> Effect {
        let order = header.clicked(self.workspace_order());
        self.changed_view_order(order)
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
    /// See `OpsWorkspaceSnapshot::activity_stale`; the effective refresh
    /// interval comes from the shown view, and it is never turned on here.
    pub(in crate::board) fn activity_stale(&self, now: std::time::Instant) -> bool {
        self.workspace_snapshot().is_none_or(|snapshot| {
            snapshot.activity_stale(self.view.as_ref().and_then(|view| view.refresh), now)
        })
    }
    /// Admits the activity clocks a painted STATE label or counter needs: the
    /// stale deadline while the read is fresh, and the core time an age counts
    /// from. No timer runs; the frame loop compares these marks.
    pub(in crate::board) fn admit_activity_clock(&self, at: Option<u64>) {
        use crate::board::view::ClockSurface;
        let deadline = self.workspace_snapshot().and_then(|snapshot| {
            snapshot.activity_deadline(self.view.as_ref().and_then(|view| view.refresh))
        });
        let mut clocks = self.frame_clocks.borrow_mut();
        if let Some(deadline) = deadline
            && !clocks
                .iter()
                .any(|clock| matches!(clock, ClockSurface::ActivityStale(_)))
        {
            clocks.push(ClockSurface::ActivityStale(deadline));
        }
        if let Some(at) = at
            && !clocks
                .iter()
                .any(|clock| matches!(clock, ClockSurface::ActivityAge(known) if *known == at))
        {
            clocks.push(ClockSurface::ActivityAge(at));
        }
    }
    /// The one STATE cell label of a painted member row; group rows have none.
    pub(in crate::board) fn activity_label(
        &self,
        row: &OpsTreeRow,
    ) -> Option<crate::board::activity::Label> {
        let label =
            self.activity_label_at(row, std::time::Instant::now(), crate::status::now_ms())?;
        self.admit_activity_clock(label.at);
        Some(label)
    }
    pub(in crate::board) fn activity_label_at(
        &self,
        row: &OpsTreeRow,
        now: std::time::Instant,
        now_ms: u64,
    ) -> Option<crate::board::activity::Label> {
        let member = row.activity.as_ref()?;
        Some(member.label(self.activity_stale(now), now_ms))
    }
    pub(in crate::board) fn tok_range(&self) -> crate::config::TokRange {
        self.workspace_snapshot()
            .map_or(crate::config::TokRange::default(), |snapshot| {
                snapshot.preferences.tok_range
            })
    }

    pub(in crate::board) fn select_tok_range(&mut self, range: crate::config::TokRange) -> Effect {
        let Some(snapshot) = self.workspace_snapshot() else {
            return self.say("The workspace has not loaded.");
        };
        let mut next = snapshot.preferences.clone();
        next.tok_range = range;
        self.workspace_save(next)
    }

    pub(in crate::board) fn cycle_tok_range(&mut self) -> Effect {
        self.select_tok_range(self.tok_range().next())
    }

    pub(in crate::board) fn digest_window(&self) -> crate::config::DigestWindow {
        self.workspace_snapshot()
            .map_or(crate::config::DigestWindow::default(), |snapshot| {
                snapshot.preferences.digest_window
            })
    }
    /// Usage and traffic are independent of the optional Digest extension. The
    /// acquired roster supplies identities; no second roster read is scheduled.
    pub(in crate::board) fn usage_report_want(
        &self,
    ) -> Option<super::super::usage_report::Request> {
        let snapshot = self.workspace_snapshot()?;
        // The Measure pane's exchange matrix reads the same report at every range.
        (self.shell.tab == OpsTab::Digest
            || self.tok_range() != crate::config::TokRange::Hour
            || (self.shell.tab == OpsTab::Agents
                && (self.tok_money() || self.right_tab == super::super::shell::RightTab::Measure)))
            .then(|| {
                super::super::usage_report::Request::new(
                    self.tok_range(),
                    std::sync::Arc::clone(&snapshot.identity_ids),
                )
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
    /// Whether the TOK figures show list-price money instead of tokens: the shared
    /// unit preference is cost, so the Digest estimates and the Agents list agree.
    pub(in crate::board) fn tok_money(&self) -> bool {
        self.savings_unit() == crate::config::SavingsUnit::Cost
    }
    /// How the Agents TOK column is headed right now.
    pub(in crate::board) fn tok_heading(&self) -> crate::board::workspace::columns::TokHeading {
        crate::board::workspace::columns::TokHeading {
            range: self.tok_range(),
            money: self.tok_money(),
        }
    }
    /// What a click on an Agents header control chooses.
    pub(in crate::board) fn control_choice(
        &self,
        control: crate::board::header_sort::Control,
    ) -> Choice {
        use crate::board::header_sort::Control;
        match control {
            Control::Sort(header) => Choice::SortBy(header),
            Control::Unit => Choice::TokUnit,
            Control::Window => Choice::TokRange(self.tok_range().next()),
        }
    }
    /// A click on a painted TOK figure switches every figure between tokens and
    /// money. It writes the shared unit preference and skips percent, which a
    /// token count cannot be.
    pub(in crate::board) fn toggle_tok_money(&mut self) -> Effect {
        let Some(snapshot) = self.workspace_snapshot() else {
            return self.say("The workspace has not loaded.");
        };
        let mut next = snapshot.preferences.clone();
        next.savings_unit = if self.tok_money() {
            crate::config::SavingsUnit::Tokens
        } else {
            crate::config::SavingsUnit::Cost
        };
        self.workspace_save(next)
    }
    /// The list-price estimate of the input tokens of `ids` in the shared TOK range, as a
    /// TOK cell: `—` until the usage report covers them completely, never a partial sum.
    pub(in crate::board) fn tok_money_text(&self, ids: &[&str]) -> String {
        use crate::board::usage_report::{View, valuation};
        let report = match self.usage_report.view_for(self.tok_range()) {
            View::Report(report) => Some(report),
            View::Loading | View::Failed(_) => None,
        };
        valuation::input_price(self, report, ids)
            .map_or_else(|| "—".into(), crate::board::savings::usd_compact)
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
        crate::board::savings::savings_caption(
            summary,
            self.digest_window(),
            self.savings_unit(),
            width,
        )
    }
    /// Whole supplied estimate for the bounded focus detail slots.
    pub(in crate::board) fn compact_savings_caption(
        &self,
        summary: &crate::labels::digest::SavingsSummary,
        width: usize,
    ) -> Option<String> {
        crate::board::savings::compact_savings_caption(
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
        crate::board::savings::savings_start(
            summary,
            self.digest_window(),
            self.local_zone.as_ref().unwrap_or(&jiff::tz::TimeZone::UTC),
            width,
        )
    }
    /// The Agents chip is active: the left pane is Overview's flat attention list of agents.
    pub(in crate::board) fn flat_agents(&self) -> bool {
        self.shell.tab == OpsTab::Agents
            && self.workspace.filter == crate::board::workspace::AgentsFilter::Agents
    }
    /// The flat list keeps its token header whole; the tree yields earlier.
    pub(in crate::board) fn agents_list_min_columns(&self) -> u16 {
        if self.flat_agents() {
            crate::board::conversation::layout::OVERVIEW_MIN_COLUMNS
        } else {
            crate::board::conversation::layout::LIST_MIN_COLUMNS
        }
    }
    pub(in crate::board) fn workspace_rows(&self) -> Rc<[OpsTreeRow]> {
        if let Some(rows) = self.frame_workspace_rows.borrow().as_ref() {
            return Rc::clone(rows);
        }
        if self.flat_agents()
            && let Some(snapshot) = self.workspace_snapshot()
        {
            return crate::board::overview::monitor::rows(self, snapshot);
        }
        if self.shell.tab == OpsTab::Digest
            && let Some(snapshot) = self.workspace_snapshot()
        {
            return crate::board::digest_dashboard::rows(self, snapshot);
        }
        self.workspace_snapshot()
            .map(|snapshot| {
                let mut rows = self.workspace.rows(&snapshot.roster, &snapshot.preferences);
                let unread = self.unread_identities();
                if !unread.is_empty() {
                    for siblings in rows.chunk_by_mut(|a, b| {
                        matches!((&a.target, &b.target),
                            (OpsTarget::Member { room_id: a, .. }, OpsTarget::Member { room_id: b, .. }) if a == b)
                    }) {
                        siblings.sort_by_key(|row| std::cmp::Reverse(matches!(&row.target,
                            OpsTarget::Member { identity_id, .. } if unread.contains(identity_id.as_str()))));
                    }
                }
                rows
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
        let rows = self.workspace_rows();
        self.workspace.selection_state(&snapshot.roster, &rows)
    }
    /// Search may retain a read-only pane, never hidden-row action authority.
    pub(in crate::board) fn workspace_retained_search_target(&self) -> Option<&OpsTarget> {
        let snapshot = self.workspace_snapshot()?;
        let rows = self.workspace_rows();
        if self.workspace.selection_state(&snapshot.roster, &rows) == SelectionState::Hidden
            && rows.is_empty()
            && !self.workspace.query.is_empty()
            && self
                .search_preview_tab
                .as_ref()
                .is_some_and(|tab| *tab == self.shell.tab)
            && matches!(self.shell.tab, OpsTab::Agents)
            && let Some(target) = self.workspace.selected.as_ref()
        {
            // An empty search keeps the last explicit pane target. Query text
            // cannot reveal a hidden/scoped-out room or bypass the Pinned mode.
            let room = match target {
                OpsTarget::Squad(room)
                | OpsTarget::Member {
                    room_id: Some(room),
                    ..
                } => Some(room),
                _ => None,
            };
            let pinned = self.shell.tab != OpsTab::Agents
                || self.workspace.filter != AgentsFilter::Pinned
                || room.is_some_and(|room| snapshot.preferences.pinned.contains(room))
                || matches!(target, OpsTarget::Member { identity_id, .. } if snapshot.preferences.agent_pins.contains(identity_id));
            let visible = room.map_or_else(
                || self.workspace.visibility.scope.is_empty(),
                |room| {
                    self.workspace
                        .visibility
                        .shows_squad(room, &snapshot.preferences)
                },
            );
            if pinned && visible {
                return Some(target);
            }
        }
        None
    }
    pub(in crate::board) fn workspace_is_pinned(&self) -> Option<bool> {
        if !matches!(self.shell.tab, OpsTab::Agents) || !self.workspace_selection_visible() {
            return None;
        }
        let preferences = &self.workspace_snapshot()?.preferences;
        match self.workspace.selected.as_ref()? {
            OpsTarget::Member { identity_id, .. } => {
                Some(preferences.agent_pins.contains(identity_id))
            }
            OpsTarget::Squad(room) => Some(preferences.pinned.contains(room)),
            OpsTarget::Pool | OpsTarget::DigestAll => None,
        }
    }

    pub(in crate::board) fn workspace_selection_visible(&self) -> bool {
        let target_shown = match self.shell.tab {
            OpsTab::Agents => self
                .overview_panels()
                .shown(crate::config::OverviewPanel::Monitor),
            OpsTab::Digest => self.digest_panels().shown(crate::config::DigestPanel::List),
        };
        target_shown && self.workspace_selection_state() == SelectionState::Visible
    }
    pub(in crate::board) fn workspace_squad(&self) -> Option<&crate::board::workspace::OpsSquad> {
        if self.flat_agents() {
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
    /// A squad selection grants a conversation only to its observed, current lead.
    pub(in crate::board) fn workspace_message_target(&self) -> Option<OpsTarget> {
        match self.workspace.selected.as_ref()? {
            OpsTarget::Squad(room)
                if matches!(self.shell.tab, OpsTab::Agents)
                    && self.workspace_selection_visible() =>
            {
                self.workspace_squad_lead(room)
            }
            member @ OpsTarget::Member { .. } if self.workspace_selection_visible() => {
                Some(member.clone())
            }
            _ => None,
        }
    }

    /// Read-only search preview admission is distinct from message authority.
    pub(in crate::board) fn workspace_preview_member(&self) -> Option<OpsTarget> {
        match self.workspace_retained_search_target()? {
            member @ OpsTarget::Member { .. } => Some(member.clone()),
            OpsTarget::Squad(room) => self.workspace_squad_lead(room),
            OpsTarget::Pool | OpsTarget::DigestAll => None,
        }
    }

    fn workspace_squad_lead(&self, room: &str) -> Option<OpsTarget> {
        let snapshot = self.workspace_snapshot()?;
        let squad = snapshot
            .roster
            .squads
            .iter()
            .find(|squad| squad.room_id == room)?;
        let lead = squad.lead.as_ref()?;
        if !squad.members.contains(lead) {
            return None;
        }
        let target = OpsTarget::Member {
            room_id: Some(room.into()),
            identity_id: lead.clone(),
        };
        (snapshot.roster.contains(&target)
            && snapshot
                .rows
                .get(&target)
                .is_some_and(|row| row["id"].as_str() == Some(lead.as_str())))
        .then_some(target)
    }
    pub(in crate::board) fn workspace_selected_member(&self) -> Option<&OpsTarget> {
        self.workspace.selected.as_ref().filter(|target| {
            matches!(target, OpsTarget::Member { .. }) && self.workspace_selection_visible()
        })
    }
    pub(in crate::board) fn workspace_scope(&self) -> OpsKeyScope {
        if self.workspace_snapshot().is_none() {
            // The generic row surface has no Overview request context.
            return if self.focused() == Pane::Replies {
                OpsKeyScope::Reader
            } else {
                OpsKeyScope::Member
            };
        }
        if self.focused_pane() == Some(Pane::Replies) {
            return OpsKeyScope::Reader;
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
    pub(in crate::board) fn workspace_focus_conversation(&mut self) {
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
        self.invalidate_frame_geometry();
    }
    pub(in crate::board) fn workspace_bindings(&self) -> Bindings {
        let _rows = self.workspace_frame_rows();
        let mut keys = self.workspace_effective_bindings();
        if self.shell.tab != OpsTab::Agents {
            keys.retain(|_, action| {
                !matches!(
                    action.verb,
                    Verb::Ui(
                        OpsUiAction::AgentsFilter
                            | OpsUiAction::NewMember
                            | OpsUiAction::DiscoverAgents
                            | OpsUiAction::RenameMember
                            | OpsUiAction::RemoveMember
                    )
                )
            });
        }
        if !self.usage_chart_available() {
            keys.retain(|_, action| action.verb != Verb::Ui(OpsUiAction::UsageControls));
        }
        if self.workspace_selected_member().is_none() {
            keys.retain(|_, action| {
                !matches!(
                    action.verb,
                    Verb::Ui(OpsUiAction::RenameMember | OpsUiAction::RemoveMember)
                )
            });
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
            Verb::Ui(OpsUiAction::Pin) => self.workspace_is_pinned().is_some(),
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
        let host = self.workspace_selected_host().filter(|host| {
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
        let host = self.workspace_selected_host();
        let mut keys = self.workspace_context_bindings(
            self.workspace.selected.as_ref(),
            self.workspace_scope(),
            host,
        );
        if let Some(sources) = self.workspace_binding_sources() {
            for (event, action) in self.workspace_factory_actions() {
                let source = sources.get(&event);
                if source.is_some_and(|source| source == "workspace default")
                    || (event == "e" && source.is_none())
                {
                    keys.insert(event, action);
                }
            }
        }
        keys
    }

    /// Context-dependent factory choices have one owner for dispatch and Settings.
    pub(super) fn workspace_factory_bindings(&self) -> Bindings {
        let mut keys = crate::config::workspace_defaults(self.workspace_scope());
        keys.extend(self.workspace_factory_actions());
        keys
    }

    fn workspace_factory_actions(&self) -> Bindings {
        let mut keys = Bindings::new();
        if self.shell.tab == OpsTab::Digest && self.focused() == Pane::Rows {
            keys.insert(
                "e".into(),
                Action::parse("monitor-details").expect("Digest details action"),
            );
        }
        if self.shell.tab == OpsTab::Agents
            && !self.flat_agents()
            && let Some(target) = self.workspace_message_target()
        {
            let pending = self
                .workspace_snapshot()
                .and_then(|snapshot| snapshot.rows.get(&target))
                .is_some_and(crate::attention::waits_on_you);
            keys.insert(
                "enter".into(),
                Action::parse(if pending { "reply" } else { "talk" })
                    .expect("factory message action"),
            );
            if let Some(action) = self.workspace_factory_j(Some(&target)) {
                keys.insert("j".into(), action);
            }
        }
        keys
    }
    /// The tree's factory j follows its recipient; the flat list keeps its scope default.
    pub(super) fn workspace_factory_j(&self, recipient: Option<&OpsTarget>) -> Option<Action> {
        if self.shell.tab == OpsTab::Agents && !self.flat_agents() && recipient.is_some() {
            Some(Action::parse("jump").expect("factory host action"))
        } else {
            crate::config::workspace_defaults(self.workspace_scope()).remove("j")
        }
    }
    fn workspace_selected_host(&self) -> Option<&str> {
        let row = self
            .workspace_message_target()
            .and_then(|target| self.workspace_snapshot()?.rows.get(&target))
            .or_else(|| self.selected_row())?;
        row["pane"]["location"]["host"].as_str()
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
        self.digest_rows.get_mut().clear();
        if self
            .search_preview_tab
            .as_ref()
            .is_some_and(|tab| *tab != self.shell.tab)
        {
            self.search_preview_tab = None;
        }
        let query_changed = self.workspace.query != self.search.as_str();
        let selects_matches = matches!(self.shell.tab, OpsTab::Agents);
        if query_changed && selects_matches {
            if self.search.is_empty() {
                self.search_preview_tab = None;
                // Search temporarily reveals collapsed matches. Clearing keeps
                // this exact occurrence visible in the full list as well.
                if self.shell.tab == OpsTab::Agents {
                    match self.workspace.selected.as_ref() {
                        Some(OpsTarget::Member {
                            room_id: Some(room),
                            ..
                        }) => {
                            self.workspace.expanded.insert(room.clone());
                        }
                        Some(OpsTarget::Member { room_id: None, .. }) => {
                            self.workspace.pool_expanded = true;
                        }
                        _ => {}
                    }
                }
                self.follow = true;
            } else if self.search_preview_tab.is_none()
                && self.workspace.selected.as_ref().is_some_and(|target| {
                    self.workspace_snapshot()
                        .is_some_and(|snapshot| snapshot.roster.contains(target))
                })
            {
                self.search_preview_tab = Some(self.shell.tab);
            }
        }
        self.workspace.query = self.search.to_string();
        if let Some(startup) = self.workspace_startup.take() {
            if let Some(id) = startup.room_id {
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
                    self.set_notice(
                        Some(format!("Scoped squad '{id}' is unavailable; choose a visible target explicitly.")),
                        false,
                    );
                }
            }
            if let Some(tab) = startup.tab {
                self.shell.tab = tab;
            }
        }
        let rows = self.workspace_rows();
        if self.flat_agents()
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
            && self.workspace.selected.as_ref().is_some_and(|target| {
                !matches!(target, OpsTarget::Member { .. } | OpsTarget::DigestAll)
            })
        {
            self.workspace.selected = rows.first().map(|row| row.target.clone());
        }
        if self.workspace.selected.is_none() {
            self.workspace.selected = rows.first().map(|row| row.target.clone());
        }
        if query_changed
            && selects_matches
            && !self.search.is_empty()
            && let Some(first) = rows.first()
        {
            self.workspace.selected = Some(first.target.clone());
        }
        if let Some(index) = rows
            .iter()
            .position(|row| Some(&row.target) == self.workspace.selected.as_ref())
        {
            self.selected = index;
        }
        // Refresh, collapse and saved visibility changes retain their anchors;
        // only an edited query above chooses its first matching target.
        self.last_click = None;
    }
    pub(super) fn workspace_select(&mut self, index: usize) {
        let rows = self.workspace_rows();
        let index = index.min(rows.len().saturating_sub(1));
        let Some(row) = rows.get(index) else {
            return;
        };
        self.workspace.selected = Some(row.target.clone());
        if self.shell.tab == OpsTab::Digest {
            // These rows already carry current admission and ordering. Cursor
            // movement changes an anchor, not the acquired workspace/query.
            self.selected = index;
            self.last_click = None;
        } else {
            self.workspace_reconcile();
        }
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
            ResizePane => {
                return self
                    .say("Use resize-pane <boundary> <direction|reset> in an authored binding.");
            }
            Quit => return Effect::Quit,
            QuotaDetails => return self.open_quota_details(),
            SavingsUnit => return self.cycle_savings_unit(),
            CycleTokRange => return self.cycle_tok_range(),
            UsageControls => return super::super::overview::chart::host::focus(self),
            Help => {
                self.help_state.borrow_mut().open();
                self.help = true;
                return Effect::None;
            }
            Agents | DigestDashboard => {
                let digit = match action {
                    Agents => '1',
                    _ => '2',
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
            TalkMeasure if self.shell.tab == OpsTab::Agents => {
                return self.select_right_tab(self.right_tab.next());
            }
            TalkMeasure => return Effect::None,
            MonitorDetails if self.shell.tab == OpsTab::Digest => {
                let Some(snapshot) = self.workspace_snapshot() else {
                    return Effect::None;
                };
                let mut next = snapshot.preferences.clone();
                let mut panels = self.digest_panels();
                if let Err(error) = panels.set_side_shown(!panels.side_shown()) {
                    return self.say(&error);
                }
                next.digest_panels = Some(crate::config::PanelSetting::configured(panels));
                return self.workspace_save(next);
            }
            MemberSession => return self.open_member_session(),
            NewMember => return self.open_new_member(),
            MonitorDetails => return self.overview_details(),
            WorkspaceSnapshots => return self.open_recovery(),
            DiscoverAgents => return self.open_discovery(),
            Previous => self.workspace_select(self.selected.saturating_sub(1)),
            Next => self.workspace_select(self.selected.saturating_add(1)),
            First => self.workspace_select(0),
            Last => self.workspace_select(usize::MAX),
            PageUp | PageDown => self.page_rows(if action == PageUp { -1 } else { 1 }),
            Search => {
                self.focus = 0;
                self.searching = true;
            }
            Fold | Expand | Inspect => {
                if !self.workspace_selection_visible() {
                    return self.say("The selected target is hidden or unavailable; select a visible target explicitly.");
                }
                let Some(target) = self.workspace.selected.clone() else {
                    return Effect::None;
                };
                if target == OpsTarget::DigestAll {
                    return Effect::None;
                }
                if matches!(target, OpsTarget::Member { .. }) {
                    if action == Inspect {
                        self.conversation_return.get_or_insert(self.shell.tab);
                        self.shell.tab = OpsTab::Agents;
                        self.right_tab = super::super::shell::RightTab::Talk;
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
                        self.focus = 1;
                        self.unread.displayed = None;
                    }
                } else {
                    let expanded = match &target {
                        OpsTarget::Pool => self.workspace.pool_expanded,
                        OpsTarget::DigestAll => false,
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
                match (action, self.flat_agents()) {
                    (Sort, true) => {
                        order.overview = Some(order.overview.map_or_else(
                            || {
                                crate::config::OverviewSort::initial(
                                    crate::config::OverviewColumn::Agent,
                                )
                            },
                            |value| value.next(),
                        ))
                    }
                    (ReverseSort, true) => {
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
                if self.workspace_is_pinned().is_none() {
                    return self.say("Select a visible agent or squad to pin.");
                }
                let Some(snapshot) = self.workspace_snapshot() else {
                    return Effect::None;
                };
                let mut next = snapshot.preferences.clone();
                match action {
                    Pin if matches!(self.workspace.selected, Some(OpsTarget::Member { .. })) => {
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
            AllTimers => return self.workspace_owner_timers(None, None),
            Timers => {
                if self.flat_agents() && self.workspace.selected.is_none() {
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
                        if self.flat_agents() {
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
            RemoveMember => return self.workspace_begin(Kind::RemoveMember),
            Manage | CreateSquad | RemoveMembership | MoveMembership => {
                return self.workspace_management(action);
            }
            ResizePane | Agents | DigestDashboard | QuotaDetails | SavingsUnit | UsageControls
            | CycleTokRange | Help | Quit | ScrollUp | ScrollDown | ScrollPageUp
            | ScrollPageDown | ScrollTop | ScrollBottom => {
                unreachable!("shell actions handled first")
            }
        }
        Effect::None
    }

    pub(in crate::board) fn agents_filter(
        &mut self,
        filter: crate::board::workspace::AgentsFilter,
    ) -> Effect {
        if self.shell.tab != OpsTab::Agents || self.workspace_scope().is_reader() {
            return Effect::None;
        }
        self.workspace.filter = filter;
        // The flat list has no squad rows: a selected squad hands over to its
        // lead, else to its first agent, else to the first row.
        if self.flat_agents()
            && let Some(OpsTarget::Squad(room)) = self.workspace.selected.clone()
        {
            let lead = self.workspace_snapshot().and_then(|snapshot| {
                snapshot
                    .roster
                    .squads
                    .iter()
                    .find(|squad| squad.room_id == room)
                    .and_then(|squad| squad.lead.clone())
            });
            let rows = self.workspace_rows();
            let member_of = |lead: Option<&String>| {
                rows.iter().find(|row| {
                    matches!(&row.target, OpsTarget::Member { room_id, identity_id }
                        if room_id.as_ref() == Some(&room)
                            && lead.is_none_or(|lead| lead == identity_id))
                })
            };
            if let Some(row) = member_of(lead.as_ref()).or_else(|| member_of(None)) {
                self.workspace.selected = Some(row.target.clone());
            }
        }
        self.workspace_reconcile();
        self.invalidate_frame_geometry();
        self.follow = true;
        Effect::None
    }
    pub(super) fn workspace_input_owner(&self) -> crate::board::shell::OpsInputOwner {
        use crate::board::shell::OpsInputOwner;
        let overlay = self.overlay();
        let editing = match overlay {
            Some(Overlay::NewMember) => true,
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
                        ),
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
        for range in crate::config::TokRange::ALL {
            entries.push(MenuEntry {
                key: String::new(),
                label: format!(
                    "TOK range: {}{}",
                    range.argument(),
                    if self.tok_range() == range {
                        " (current)"
                    } else {
                        ""
                    }
                ),
                choice: Choice::TokRange(range),
            });
        }

        entries.push(MenuEntry {
            key: String::new(),
            label: "Bindings · j presets".into(),
            choice: Choice::HostAction,
        });
        entries.push(MenuEntry {
            key: String::new(),
            label: "Launch templates…".into(),
            choice: Choice::LaunchTemplates,
        });
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
        if matches!(self.shell.tab, OpsTab::Agents | OpsTab::Digest) {
            entries.push(MenuEntry {
                key: String::new(),
                label: "Panels".into(),
                choice: Choice::Panels,
            });
        }
        if let Some(squad) = self.workspace_squad().map(|squad| squad.name.clone()) {
            // The menu is 48 cells wide and truncates labels, so a long rule fills any width.
            entries.push(MenuEntry {
                key: String::new(),
                label: "─".repeat(48),
                choice: Choice::Rule,
            });
            entries.push(MenuEntry {
                key: String::new(),
                label: format!(
                    "Squad settings: {}",
                    crate::board::workspace::render::safe(&squad)
                ),
                choice: Choice::SquadSettings(squad),
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
    /// Selects the Agents tab's right pane; choosing the pane already shown changes nothing.
    pub(in crate::board) fn select_right_tab(
        &mut self,
        tab: super::super::shell::RightTab,
    ) -> Effect {
        if self.shell.tab == OpsTab::Agents && self.right_tab != tab {
            self.right_tab = tab;
            self.invalidate_frame_geometry();
        }
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
        self.set_notice(Some(message.clone()), false);
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
