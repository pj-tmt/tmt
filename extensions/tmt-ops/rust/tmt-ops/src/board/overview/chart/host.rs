//! One session chart controller shared by Overview and Digest. Source rings and
//! frame diffs remain with their existing owners; this cache holds only 12 totals.
use super::{detail::Details, model::Selection, project, scene::Scene};
use crate::{
    action::{OpsUiAction, Verb},
    board::{
        app::{App, Effect},
        rate::series::{self, Grid, Series},
        shell::OpsTab,
        workspace::OpsObservation,
    },
    config::Pane,
};
use ratatui::{
    Frame,
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    text::Line,
};
use std::{collections::BTreeSet, rc::Rc};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

mod controls;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Each,
    Range,
    Bars,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Each(u8),
    Range(u8),
    Bar(usize),
}
#[derive(Clone, Copy)]
pub(super) struct Hit {
    pub area: Rect,
    pub target: Target,
}

#[derive(Default)]
pub(in crate::board) struct State {
    selection: Selection,
    focus: Option<Focus>,
    draft: usize,
    bar: usize,
    hits: Vec<Hit>,
    visible: bool,
    hover: Option<usize>,
    observed: Option<Rc<Series>>,
    // The scene key includes all display values, coverage and solved dimensions.
    scene: Option<(super::model::Series, u16, u8, Scene)>,
}
impl State {
    pub fn release_focus(&mut self) {
        self.focus = None;
        self.hover = None;
    }
    pub fn focused(&self) -> bool {
        self.focus.is_some()
    }
    pub fn invalidate_geometry(&mut self) {
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
            Focus::Each => Selection::EACH
                .iter()
                .position(|v| *v == self.selection.each_minutes)
                .unwrap(),
            Focus::Range => Selection::RANGE
                .iter()
                .position(|v| *v == self.selection.range_minutes)
                .unwrap(),
            Focus::Bars => self.bar,
        };
    }
    fn apply(&mut self, target: Target) -> Result<(), &'static str> {
        let mut next = self.selection;
        match target {
            Target::Each(v) => next.each_minutes = v,
            Target::Range(v) => next.range_minutes = v,
            Target::Bar(_) => return Ok(()),
        }
        self.selection.change(next)?;
        self.bar = self.bar.min(next.bars()? - 1);
        self.observed = None;
        self.scene = None;
        self.hover = None;
        self.hits.clear();
        Ok(())
    }
}
impl App {
    pub(in crate::board) fn usage_chart_available(&self) -> bool {
        self.workspace_snapshot().is_some()
            && matches!(self.shell.tab, OpsTab::Overview | OpsTab::Digest)
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
        state.observed = Some(Rc::new(app.usage_series(state.selection, now)));
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
    if state.visible && changes_with_time && !app.modal_open() {
        format!("{}", now / crate::board::rate::SLOT_MS)
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
    if !app.usage_chart.borrow().visible {
        return app
            .say("The terminal is too short for chart controls; enlarge it to inspect usage.");
    }
    app.usage_chart.borrow_mut().set_focus(Focus::Each);
    Effect::None
}
fn inspect(app: &mut App, index: usize) -> Effect {
    let lines = {
        let mut state = app.usage_chart.borrow_mut();
        refresh(app, &mut state, crate::status::now_ms());
        state.bar = index.min(state.selection.bars().unwrap() - 1);
        project::details(&state.observed.as_ref().unwrap().buckets[state.bar])
    };
    app.usage_details = Some(std::cell::RefCell::new(Details::new(lines)));
    app.invalidate_frame_geometry();
    Effect::None
}
/// The parent forwards only after composers, search, readers and overlays own input.
pub(in crate::board) fn key(app: &mut App, key: KeyEvent) -> Option<Effect> {
    if !app.usage_chart_available() {
        app.usage_chart.borrow_mut().focus = None;
        return None;
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
        Focus::Each => Selection::EACH.len(),
        Focus::Range => Selection::RANGE.len(),
        Focus::Bars => state.selection.bars().unwrap(),
    };
    match key.code {
        KeyCode::Esc => state.focus = None,
        KeyCode::Tab => state.set_focus(match focus {
            Focus::Each => Focus::Range,
            Focus::Range => Focus::Bars,
            Focus::Bars => Focus::Each,
        }),
        KeyCode::Left | KeyCode::Up => {
            state.draft = state.draft.saturating_sub(1);
            if focus == Focus::Bars {
                state.bar = state.draft;
            }
        }
        KeyCode::Right | KeyCode::Down => {
            state.draft = (state.draft + 1).min(count - 1);
            if focus == Focus::Bars {
                state.bar = state.draft;
            }
        }
        KeyCode::Enter => {
            let target = match focus {
                Focus::Each => Target::Each(Selection::EACH[state.draft]),
                Focus::Range => Target::Range(Selection::RANGE[state.draft]),
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
            state.focus = None;
            return None;
        }
    }
    Some(Effect::None)
}
pub(in crate::board) fn mouse(app: &mut App, event: MouseEvent) -> Option<Effect> {
    if !app.usage_chart_available() || app.loading() {
        return None;
    }
    let target = app
        .usage_chart
        .borrow()
        .hits
        .iter()
        .find(|hit| hit.area.contains((event.column, event.row).into()))
        .map(|hit| hit.target);
    if event.kind == MouseEventKind::Moved {
        app.usage_chart.borrow_mut().hover = match target {
            Some(Target::Bar(index)) => Some(index),
            _ => None,
        };
        return target.map(|_| Effect::None);
    }
    if event.kind != MouseEventKind::Down(MouseButton::Left) {
        return None;
    }
    match target? {
        Target::Bar(index) => {
            app.usage_chart.borrow_mut().set_focus(Focus::Bars);
            Some(inspect(app, index))
        }
        target => {
            let result = app.usage_chart.borrow_mut().apply(target);
            Some(result.map_or_else(|message| app.say(message), |_| Effect::None))
        }
    }
}

pub(in crate::board) fn render(frame: &mut Frame, app: &App, mut area: Rect) -> Rect {
    let look = app.look(); // Resolve bindings/theme before borrowing controller state.
    let key = app
        .workspace_bindings()
        .into_iter()
        .find(|(_, action)| action.verb == Verb::Ui(OpsUiAction::UsageControls))
        .map(|(key, _)| key);
    let mut state = app.usage_chart.borrow_mut();
    state.hits.clear();
    state.visible = false;
    // Never consume the usable target list to make room for decoration.
    if area.width < 38 || area.height < 9 {
        state.hover = None;
        return area;
    }
    state.visible = true;
    refresh(app, &mut state, crate::status::now_ms());
    let data = Rc::clone(state.observed.as_ref().unwrap());
    let active = state.focus.filter(|_| app.focused() == Pane::Rows);
    let title = format!(
        "TOK / {}m · unique identities{}",
        state.selection.each_minutes,
        key.map_or(String::new(), |key| format!(" · {key} controls"))
    );
    strip::paint_left(
        frame.buffer_mut(),
        Rect::new(area.x, area.y, area.width, 1),
        Line::styled(title, look.role(Role::Muted)),
    );
    let controls_area = Rect::new(area.x, area.y + 1, area.width, 2);
    let controls = controls::paint(
        frame.buffer_mut(),
        controls_area,
        state.selection,
        active,
        state.draft,
        look,
    );
    state.hits.extend(controls);
    let partial = data.buckets.iter().any(|bucket| !bucket.complete());
    if area.height < 11 + u16::from(partial) {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, area.y + 3, area.width, 1),
            Line::styled(
                "Chart hidden at this height · enlarge for interval details",
                look.role(Role::Muted),
            ),
        );
        area.y += 4;
        area.height -= 4;
        if active == Some(Focus::Bars) {
            state.focus = Some(Focus::Each);
        }
        return area;
    }
    let plot_height = area
        .height
        .saturating_sub(10 + u16::from(partial))
        .clamp(1, 5) as u8;
    let plot = Rect::new(
        area.x,
        area.y + 3,
        area.width,
        u16::from(plot_height) + 1 + u16::from(partial),
    );
    let display = project::display(&data, state.selection).expect("validated interval projection");
    if state.scene.as_ref().is_none_or(|(old, width, height, _)| {
        old != &display || *width != plot.width || *height != plot_height
    }) {
        let scene = Scene::new(&display, plot.width, plot_height).expect("bounded chart scene");
        state.scene = Some((display, plot.width, plot_height, scene));
    }
    // Scene solves/paints the actual sub-rectangle. Copy its bounded cells into
    // the existing Frame buffer; Ratatui retains the sole terminal diff owner.
    let mut buffer = Buffer::empty(plot);
    let hits = state
        .scene
        .as_ref()
        .unwrap()
        .3
        .paint(&mut buffer, &look.theme, look.depth)
        .expect("admitted usage chart");
    let owned_hits: Vec<_> = hits
        .iter()
        .filter_map(|hit| {
            let id = hit.id?.last()?;
            let (_, suffix) = id.rsplit_once("bar-")?;
            let index = suffix.parse::<usize>().ok()?;
            let rect = hit.rect;
            Some(Hit {
                area: Rect::new(
                    rect.x.try_into().ok()?,
                    rect.y.try_into().ok()?,
                    rect.width.try_into().ok()?,
                    rect.height.try_into().ok()?,
                ),
                target: Target::Bar(index),
            })
        })
        .collect();
    state.hits.extend(owned_hits);
    if active == Some(Focus::Bars) {
        for hit in &state.hits {
            if hit.target == Target::Bar(state.bar) && hit.area.y == plot.y + u16::from(plot_height)
            {
                buffer.set_style(hit.area, look.selection());
            }
        }
    }
    for y in plot.y..plot.bottom() {
        for x in plot.x..plot.right() {
            frame.buffer_mut()[(x, y)] = buffer[(x, y)].clone();
        }
    }
    let selected = state.hover.or(active
        .filter(|focus| *focus == Focus::Bars)
        .map(|_| state.bar));
    let label = selected.map_or_else(
        || project::range_label(&data, state.selection.range_minutes),
        |index| project::details(&data.buckets[index]).join(" · "),
    );
    strip::paint_left(
        frame.buffer_mut(),
        Rect::new(area.x, plot.bottom(), area.width, 1),
        Line::styled(label, look.role(Role::Muted)),
    );
    let used = plot.bottom() + 1 - area.y;
    area.y += used;
    area.height -= used;
    if !app.modal_open() {
        app.frame_clocks
            .borrow_mut()
            .push(crate::board::view::ClockSurface::UsageChart);
    }
    area
}
