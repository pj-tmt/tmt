//! One session chart controller shared by Overview and Digest. Source rings and
//! frame diffs remain with their existing owners; this cache holds bounded totals.
use super::{
    model::{Each, Selection},
    project,
    viewport::Viewport,
};
use crate::board::spark_row::Scene;
use crate::{
    board::{
        app::{App, Effect},
        rate::series::{self, Grid, Series},
        shell::OpsTab,
        workspace::OpsObservation,
    },
    config::Pane,
};
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};
use std::{collections::BTreeSet, rc::Rc};

mod controls;
mod dropdown;
mod render;
pub(in crate::board) use render::{demand, overview_demand, render_controls, render_panel};
#[cfg(test)]
pub(in crate::board) mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Each,
    Range,
    Quota,
    Bars,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    OpenEach,
    OpenRange,
    Each(Each),
    Range(u32),
    Quota(crate::board::limits::QuotaWindow),
    Bar(usize),
    Controls,
}
#[derive(Clone, Copy)]
pub(super) struct Hit {
    pub area: Rect,
    pub target: Target,
}

#[derive(Default)]
pub(in crate::board) struct State {
    dropdown: Option<dropdown::Dropdown>,
    selection: Selection,
    each: Each,
    viewport: Viewport,
    pinned: Option<i64>,
    quota_window: crate::board::limits::QuotaWindow,
    focus: Option<Focus>,
    draft: usize,
    bar: usize,
    hits: Vec<Hit>,
    visible: bool,
    hover: Option<usize>,
    observed: Option<Rc<Series>>,
    // The scene key includes all display values, coverage and solved dimensions.
    scene: Option<(crate::board::spark_row::Data, u16, u64, Scene)>,
}
impl State {
    pub(in crate::board) fn chart_controls(&self) -> (u32, Each) {
        (self.selection.range_minutes, self.each)
    }
    pub(in crate::board) fn restore_chart_controls(&mut self, range: u32, each: Each) {
        if Selection::RANGE.contains(&range) {
            self.each = each;
            self.selection = each.resolve(range, self.viewport.width);
            self.reset_series();
        }
    }
    fn reset_series(&mut self) {
        self.observed = None;
        self.scene = None;
        self.hover = None;
        self.pinned = None;
        self.viewport.reset();
        self.bar = self.selection.bars().expect("admitted selection") - 1;
        self.hits.clear();
    }
    fn resolve_width(&mut self, width: usize) {
        self.viewport.width = width;
        if let Some(dropdown) = &mut self.dropdown {
            dropdown.resolve_width(self.selection.range_minutes, width);
        }
        let next = self.each.resolve(self.selection.range_minutes, width);
        if next != self.selection {
            self.selection = next;
            self.reset_series();
        }
    }
    pub(in crate::board) fn quota_window(&self) -> crate::board::limits::QuotaWindow {
        self.quota_window
    }

    pub(in crate::board) fn restore_quota_window(
        &mut self,
        window: crate::board::limits::QuotaWindow,
    ) {
        self.quota_window = window;
        self.invalidate_geometry();
    }

    pub(in crate::board) fn retained_request(
        &self,
    ) -> Option<crate::board::rate::history::rollup::Request> {
        crate::board::rate::history::rollup::Request::new(
            u64::from(self.selection.each_minutes) * 60_000,
            u64::from(self.selection.range_minutes) * 60_000,
        )
    }
    pub fn release_focus(&mut self) {
        self.focus = None;
        self.hover = None;
        self.dropdown = None;
    }
    pub(in crate::board) fn scrollable(&self) -> bool {
        self.selection.bars().is_ok_and(|n| n > self.viewport.width)
    }
    pub fn focused(&self) -> bool {
        self.focus.is_some()
    }
    pub fn invalidate_geometry(&mut self) {
        if let Some(dropdown) = &mut self.dropdown {
            dropdown.invalidate();
        }
        self.hits.clear();
        self.visible = false;
        self.hover = None;
    }
    pub fn invalidate_data(&mut self) {
        self.observed = None;
        self.invalidate_geometry();
    }
    fn set_focus(&mut self, focus: Focus) {
        self.focus = Some(focus);
        self.draft = match focus {
            Focus::Each => match self.each {
                Each::Auto => 0,
                Each::Fixed(each) => {
                    Selection::each_choices(self.selection.range_minutes)
                        .iter()
                        .position(|v| *v == each)
                        .unwrap_or(0)
                        + 1
                }
            },
            Focus::Range => Selection::RANGE
                .iter()
                .position(|v| *v == self.selection.range_minutes)
                .unwrap(),
            Focus::Quota => crate::board::limits::QuotaWindow::ALL
                .iter()
                .position(|v| *v == self.quota_window)
                .unwrap(),
            Focus::Bars => self.bar,
        };
    }
    fn apply(&mut self, target: Target) -> Result<(), &'static str> {
        let mut next = self.selection;
        match target {
            Target::Quota(window) => {
                self.quota_window = window;
                self.hits.clear();
                self.hover = None;
                if self.focus == Some(Focus::Quota) {
                    self.set_focus(Focus::Quota);
                }
                return Ok(());
            }
            Target::Each(Each::Auto) => {
                self.each = Each::Auto;
                next = self.each.resolve(next.range_minutes, self.viewport.width);
            }
            Target::Each(Each::Fixed(v)) => {
                next.each_minutes = v;
                next.validate()?;
                self.each = Each::Fixed(v);
            }
            Target::Range(v) => {
                next = next.with_range(v)?;
                if matches!(self.each, Each::Fixed(_)) {
                    self.each = Each::Fixed(next.each_minutes);
                }
                next = self.each.resolve(v, self.viewport.width);
            }
            Target::Bar(_) | Target::Controls | Target::OpenEach | Target::OpenRange => {
                return Ok(());
            }
        }
        next.validate()?;
        if next != self.selection {
            self.selection.change(next)?;
            self.reset_series();
        }
        if let Some(focus) = self.focus {
            self.set_focus(focus);
        }
        Ok(())
    }
}
impl App {
    pub(in crate::board) fn retained_history_request(
        &self,
    ) -> Option<crate::board::rate::history::rollup::Request> {
        if !matches!(self.shell.tab, OpsTab::Agents | OpsTab::Digest)
            || self.workspace_snapshot().is_none()
            || self.usage_panel_hidden()
        {
            None
        } else {
            self.usage_chart.borrow().retained_request()
        }
    }
    pub(in crate::board) fn quota_history_visible(&self) -> bool {
        self.shell.tab == OpsTab::Agents
            && self.workspace_snapshot().is_some()
            && !self.usage_panel_hidden()
            && render::quotas_visible(self)
    }
    pub(in crate::board) fn history_demand(&self) -> Option<crate::board::rate::history::Demand> {
        let chart = self.retained_history_request();
        let quota = self.quota_history_visible();
        (chart.is_some() || quota).then_some(crate::board::rate::history::Demand { chart, quota })
    }
    pub(in crate::board) fn usage_chart_available(&self) -> bool {
        self.workspace_snapshot().is_some()
            && matches!(self.shell.tab, OpsTab::Agents | OpsTab::Digest)
            && self.focused() == Pane::Rows
    }
    fn usage_series(&self, selection: Selection, now: u64) -> Series {
        let grid = Grid::new(
            u64::from(selection.each_minutes) * 60_000,
            u64::from(selection.range_minutes) * 60_000,
            now,
        )
        .expect("admitted chart selection and clock");
        let roster = &self
            .workspace_snapshot()
            .expect("chart has a workspace")
            .roster;
        let expected: BTreeSet<_> = roster.members.keys().map(String::as_str).collect();
        // Acquired, enabled sources only. Missing common-pool or disabled sources
        // remain missing members of the scope; they never shrink it to a zero.
        let rates = self
            .view
            .as_ref()
            .into_iter()
            .flat_map(|view| &view.home_rate)
            .filter(|(_, rate)| rate.settings.enabled)
            .filter_map(|(name, _)| self.home_meter(name))
            .map(|meter| meter.history());
        let mut observed = series::project(rates, grid, Some(&expected));
        for bucket in &mut observed.buckets {
            bucket.scope_complete =
                roster.observation == OpsObservation::Complete && roster.membership_complete;
        }
        observed
    }
}
fn refresh(app: &App, state: &mut State, now: u64) {
    let grid = Grid::new(
        u64::from(state.selection.each_minutes) * 60_000,
        u64::from(state.selection.range_minutes) * 60_000,
        now,
    )
    .expect("admitted chart clock");
    if state.observed.as_ref().is_none_or(|old| old.grid != grid) {
        let observed = app.usage_series(state.selection, now);
        if state
            .pinned
            .is_some_and(|from| !observed.buckets.iter().any(|bucket| bucket.from_ms == from))
        {
            state.pinned = None;
        }
        state.observed = Some(Rc::new(observed));
    }
}
pub(in crate::board) fn clock_mark(app: &App, now: u64) -> String {
    let state = app.usage_chart.borrow();
    let changes_with_time = state.hover.is_some()
        || state.focus == Some(Focus::Bars)
        || state
            .observed
            .as_ref()
            .is_some_and(|source| source.buckets.iter().any(|bucket| bucket.tokens.is_some()));
    if state.visible && changes_with_time && (!app.modal_open() || app.usage_controls_active()) {
        format!(
            "{}",
            now / if state.selection.range_minutes > 60 {
                crate::board::rate::history::rollup::RESOLUTION_MS
            } else {
                crate::board::rate::SLOT_MS
            }
        )
    } else {
        String::new()
    }
}
pub(in crate::board) fn focus(app: &mut App) -> Effect {
    if !app.usage_chart_available() || app.loading() {
        return app.say(
            "Usage chart controls are available in the Overview and Digest lists after loading.",
        );
    }
    // Overview may omit its inline chart to keep the last usable primary row.
    // Its freshly painted body still admits the read-only dropdown triggers;
    // a pre-frame or invalidated workspace never bypasses the visibility fence.
    let overview_table = app.shell.tab == OpsTab::Agents && app.scrolls.visible(Pane::Rows);
    if !app.usage_chart.borrow().visible {
        if app.shell.tab == OpsTab::Digest
            && app.scrolls.visible(Pane::Rows)
            && !app.usage_panel_hidden()
        {
            app.usage_chart.borrow_mut().set_focus(Focus::Range);
            return Effect::None;
        }
        if overview_table {
            return open_dropdown(app, Focus::Range);
        }
        return app.say("Usage controls require a visible workspace frame.");
    }
    if app.in_flight > 0 {
        return app.say("Wait for the pending action before opening usage controls.");
    }
    app.usage_chart.borrow_mut().set_focus(Focus::Range);
    Effect::None
}
fn inspect(app: &mut App, index: usize) -> Effect {
    let mut state = app.usage_chart.borrow_mut();
    refresh(app, &mut state, crate::status::now_ms());
    state.bar = index.min(state.selection.bars().unwrap() - 1);
    let from = state.observed.as_ref().unwrap().buckets[state.bar].from_ms;
    state.pinned = (state.pinned != Some(from)).then_some(from);
    Effect::None
}
fn open_dropdown(app: &mut App, focus: Focus) -> Effect {
    let mut state = app.usage_chart.borrow_mut();
    state.set_focus(focus);
    state.hover = None;
    state.dropdown = Some(dropdown::Dropdown::new(&state, focus));
    drop(state);
    app.usage_controls = true;
    Effect::None
}
fn commit_dropdown(app: &mut App) -> Effect {
    let selected = app.usage_chart.borrow().dropdown.as_ref().unwrap().choose();
    match selected {
        Err(reason) => app.say(reason),
        Ok(target) => {
            let result = app.usage_chart.borrow_mut().apply(target);
            if let Err(reason) = result {
                return app.say(reason);
            }
            app.usage_chart.borrow_mut().dropdown = None;
            app.usage_controls = false;
            Effect::None
        }
    }
}
/// The parent forwards only after composers, search, readers and overlays own input.
pub(in crate::board) fn key(app: &mut App, key: KeyEvent) -> Option<Effect> {
    if !app.usage_chart_available() {
        app.usage_chart.borrow_mut().focus = None;
        return None;
    }
    // An inactive chart cannot own this event. Resolve selected-row binding
    // provenance only when a chart control or pinned Escape can consume it.
    if !app.usage_controls
        && app.usage_chart.borrow().focus.is_none()
        && app.usage_chart.borrow().hover.is_none()
        && app.usage_chart.borrow().dropdown.is_none()
        && !(key.code == KeyCode::Esc && app.usage_chart.borrow().pinned.is_some())
    {
        return None;
    }
    if !app.usage_controls
        && crate::board::app::event_name(key)
            .as_deref()
            .is_some_and(|event| {
                app.workspace_binding_sources()
                    .and_then(|sources| sources.get(event))
                    .is_some_and(|source| source != "workspace default")
            })
    {
        app.usage_chart.borrow_mut().release_focus();
        return None;
    }
    if app.usage_controls {
        if key.code == KeyCode::Enter {
            return Some(commit_dropdown(app));
        }
        if let Some(dropdown) = &mut app.usage_chart.borrow_mut().dropdown {
            dropdown.key(key.code);
        }
        return Some(Effect::None);
    }
    if key.code == KeyCode::Esc && app.usage_chart.borrow().pinned.is_some() {
        let mut state = app.usage_chart.borrow_mut();
        state.pinned = None;
        state.hover = None;
        state.focus = None;
        return Some(Effect::None);
    }
    let focus = app.usage_chart.borrow().focus?;
    if !app.usage_chart.borrow().visible {
        app.usage_chart.borrow_mut().focus = None;
        return Some(Effect::None);
    }
    // Explicit bindings still win for non-navigation keys, including a rebound u.
    if key.modifiers != KeyModifiers::NONE {
        return None;
    }
    let mut state = app.usage_chart.borrow_mut();
    let count = match focus {
        Focus::Each | Focus::Range => 1,
        Focus::Quota => crate::board::limits::QuotaWindow::ALL.len(),
        Focus::Bars => state.selection.bars().unwrap(),
    };
    match key.code {
        KeyCode::Esc => {
            state.pinned = None;
            state.focus = None;
            state.hover = None;
        }
        KeyCode::Home | KeyCode::End if focus == Focus::Bars => {
            state.bar = if key.code == KeyCode::Home {
                0
            } else {
                count - 1
            };
            state.draft = state.bar;
            if key.code == KeyCode::Home {
                state.viewport.oldest();
            } else {
                state.viewport.newest();
            }
        }
        KeyCode::Tab => {
            let next = match focus {
                Focus::Range => Focus::Each,
                Focus::Each
                    if state
                        .hits
                        .iter()
                        .any(|hit| matches!(hit.target, Target::Quota(_))) =>
                {
                    Focus::Quota
                }
                Focus::Each | Focus::Quota => Focus::Bars,
                Focus::Bars => Focus::Range,
            };
            state.set_focus(next);
        }
        KeyCode::Left | KeyCode::Up => {
            state.draft = state.draft.saturating_sub(1);
            if focus == Focus::Bars {
                state.bar = state.draft;
                let bar = state.bar;
                state.viewport.reveal(count, bar);
            }
        }
        KeyCode::Right | KeyCode::Down => {
            state.draft = (state.draft + 1).min(count - 1);
            if focus == Focus::Bars {
                state.bar = state.draft;
                let bar = state.bar;
                state.viewport.reveal(count, bar);
            }
        }
        KeyCode::Enter => {
            let target = match focus {
                Focus::Each | Focus::Range => {
                    drop(state);
                    return Some(open_dropdown(app, focus));
                }
                Focus::Quota => Target::Quota(crate::board::limits::QuotaWindow::ALL[state.draft]),
                Focus::Bars => Target::Bar(state.bar),
            };
            if let Target::Bar(index) = target {
                drop(state);
                return Some(inspect(app, index));
            }
            let result = state.apply(target);
            drop(state);
            if let Err(message) = result {
                return Some(app.say(message));
            }
            return Some(Effect::None);
        }
        _ => {
            if app.usage_controls {
                return Some(Effect::None);
            }
            state.focus = None;
            return None;
        }
    }
    Some(Effect::None)
}
/// The session loop uses this semantic change to admit a repaint on motion.
pub(in crate::board) fn pointer(app: &mut App, event: MouseEvent) -> bool {
    if !app.usage_chart_available() || app.loading() {
        return false;
    }
    let mut state = app.usage_chart.borrow_mut();
    if app.usage_controls_active() {
        return state
            .dropdown
            .as_mut()
            .is_some_and(|menu| menu.pointer(event));
    }
    let hover = state
        .hits
        .iter()
        .find(|hit| hit.area.contains((event.column, event.row).into()))
        .and_then(|hit| match hit.target {
            Target::Bar(index) => Some(index),
            _ => None,
        });
    let changed = state.hover != hover;
    state.hover = hover;
    changed
}

pub(in crate::board) fn mouse(app: &mut App, event: MouseEvent) -> Option<Effect> {
    if !app.usage_chart_available() || app.loading() {
        return None;
    }
    if app.usage_controls {
        let choose = app
            .usage_chart
            .borrow_mut()
            .dropdown
            .as_mut()
            .is_some_and(|d| d.mouse(event));
        return Some(if choose {
            commit_dropdown(app)
        } else {
            Effect::None
        });
    }
    let target = app
        .usage_chart
        .borrow()
        .hits
        .iter()
        .find(|hit| hit.area.contains((event.column, event.row).into()))
        .map(|hit| hit.target);
    if matches!(target, Some(Target::Bar(_)))
        && matches!(
            event.kind,
            MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        )
    {
        let mut state = app.usage_chart.borrow_mut();
        let total = state.selection.bars().unwrap();
        let delta = if matches!(
            event.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft
        ) {
            -1
        } else {
            1
        };
        state.viewport.scroll(total, delta);
        state.hover = None;
        let window = state.viewport.window(total);
        if state.focus == Some(Focus::Bars) {
            state.bar = state.bar.clamp(window.start, window.end - 1);
            state.draft = state.bar;
        }
        return Some(Effect::None);
    }
    if event.kind != MouseEventKind::Down(MouseButton::Left) {
        return None;
    }
    match target? {
        Target::OpenEach => Some(open_dropdown(app, Focus::Each)),
        Target::OpenRange => Some(open_dropdown(app, Focus::Range)),
        Target::Controls => Some(focus(app)),
        Target::Bar(index) => Some(inspect(app, index)),
        target => {
            let result = app.usage_chart.borrow_mut().apply(target);
            Some(result.map_or_else(|message| app.say(message), |_| Effect::None))
        }
    }
}
