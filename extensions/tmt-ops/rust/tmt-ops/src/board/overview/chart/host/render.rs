//! Inline and modal placement share one chart painter and semantic scene cache.
use super::*;
use ratatui::{Frame, buffer::Buffer, text::Line};
use tmt_cli_style::Role;
use tmt_tui::components::{Modal, Placement, strip};

fn list_capacity(app: &App, area: Rect) -> usize {
    let Some(placement) = app.workspace_placement(area) else {
        return 0;
    };
    let Some(mut list) = placement.list else {
        return 0;
    };
    // Reserve the existing outline and Digest's table header. Overview also
    // reserves a monitor header so the M01 host can integrate without squeezing rows.
    list.height = list.height.saturating_sub(3);
    crate::board::scroll::Scrolls::viewport(list, app.workspace_rows().len())
}
fn hint(app: &App) -> Option<String> {
    app.workspace_bindings()
        .into_iter()
        .find(|(_, action)| action.verb == Verb::Ui(OpsUiAction::UsageControls))
        .map(|(key, _)| key)
}
fn chart_height(app: &App, area: Rect) -> Option<u8> {
    (1..=5).rev().find(|height| {
        let used = u16::from(*height) + 3 + controls::height(area.width);
        area.height >= used
            && list_capacity(
                app,
                Rect::new(area.x, area.y + used, area.width, area.height - used),
            ) >= 6
    })
}
/// Pure paint demand from held samples and the available chart/target budget.
/// No chart visibility/cache, hit map, focus or clock mark is changed here.
/// The existing clock is captured once and shared with the final painter.
pub(in crate::board) fn demand(app: &App, width: u16, target_row_budget: u16, now: u64) -> u16 {
    if width == 0 || target_row_budget <= 1 || app.workspace_snapshot().is_none() {
        return 0;
    }
    let state = app.usage_chart.borrow();
    let grid = Grid::new(
        u64::from(state.selection.each_minutes) * 60_000,
        u64::from(state.selection.range_minutes) * 60_000,
        now,
    )
    .expect("admitted chart clock");
    let positive = if let Some(data) = state.observed.as_ref().filter(|data| data.grid == grid) {
        data.buckets
            .iter()
            .any(|bucket| bucket.tokens.is_some_and(|n| n > 0))
    } else {
        app.usage_series(state.selection, now)
            .buckets
            .iter()
            .any(|bucket| bucket.tokens.is_some_and(|n| n > 0))
    };
    let area = Rect::new(0, 0, width, target_row_budget);
    if positive
        && width >= 38
        && !app.usage_controls
        && let Some(height) = chart_height(app, area)
    {
        return u16::from(height) + 3 + controls::height(width);
    }
    u16::from(list_capacity(app, Rect::new(0, 1, width, target_row_budget - 1)) > 0)
}

/// Legacy tail callers use exactly the same demand and own-rectangle painter.
pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) -> Rect {
    let now = crate::status::now_ms();
    let used = demand(app, area.width, area.height, now).min(area.height);
    render_panel(
        frame,
        app,
        Rect {
            height: used,
            ..area
        },
        now,
    );
    Rect {
        y: area.y + used,
        height: area.height - used,
        ..area
    }
}

pub(in crate::board) fn render_panel(frame: &mut Frame, app: &App, area: Rect, now: u64) {
    let look = app.look();
    let key = hint(app);
    let mut state = app.usage_chart.borrow_mut();
    state.hits.clear();
    state.visible = false;
    // Do not project before acquisition, or displace the last usable target row.
    if area.is_empty() || app.workspace_snapshot().is_none() {
        state.hover = None;
        return;
    }
    state.visible = true;
    refresh(app, &mut state, now);
    let data = Rc::clone(state.observed.as_ref().unwrap());
    // Unknown/zero data remains honest and reachable through the same controls,
    // without reserving an empty plot or exposing meaningless bucket hit areas.
    let positive = data
        .buckets
        .iter()
        .any(|bucket| bucket.tokens.is_some_and(|n| n > 0));
    let height = (positive && area.width >= 38 && !app.usage_controls)
        .then(|| {
            (1..=5)
                .rev()
                .find(|height| u16::from(*height) + 3 + controls::height(area.width) <= area.height)
        })
        .flatten();
    if let Some(height) = height {
        paint_chart(frame, app, &mut state, area, look, key, height);
    } else {
        if !app.usage_controls {
            state.release_focus();
        }
        let label = super::super::format::range_label(&data, state.selection.range_minutes);
        let reading = label
            .split_once(" · ")
            .map_or(label.as_str(), |(_, reading)| reading);
        let reading = if app.view.as_ref().is_some_and(|view| view.history_pending) {
            "Updating usage…"
        } else {
            reading
        };
        let summary = format!(
            "TOK / {}m · {reading}{}",
            state.selection.each_minutes,
            key.map_or(String::new(), |key| format!(" · {key} chart"))
        );
        state.hits = controls::summary(
            frame.buffer_mut(),
            Rect::new(area.x, area.y, area.width, 1),
            state.selection,
            &summary,
            look,
        );
    }
    if !app.modal_open() {
        app.frame_clocks
            .borrow_mut()
            .push(crate::board::view::ClockSurface::UsageChart);
    }
}
pub(in crate::board) fn render_controls(frame: &mut Frame, app: &App, body: Rect) {
    let look = app.look();
    let modal = Modal {
        title: "Usage · Each / Range".into(),
        placement: Placement::Body,
    };
    let areas = modal.areas(body, [body.width, body.height], true, false);
    modal.paint_flat(areas, frame.buffer_mut(), &look.theme, look.depth);
    let mut state = app.usage_chart.borrow_mut();
    state.hits.clear();
    state.visible = false;
    refresh(app, &mut state, crate::status::now_ms());
    let partial = state
        .observed
        .as_ref()
        .unwrap()
        .buckets
        .iter()
        .any(|bucket| !bucket.complete());
    if areas.content.height >= 2 {
        state.visible = true;
        if state.focus.is_none() {
            state.set_focus(Focus::Each);
        }
        let required = 7 + u16::from(partial);
        if areas.content.height >= required {
            let height = areas
                .content
                .height
                .saturating_sub(6 + u16::from(partial))
                .clamp(1, 5) as u8;
            paint_chart(frame, app, &mut state, areas.content, look, None, height);
        } else {
            state.hits = controls::paint(
                frame.buffer_mut(),
                Rect {
                    height: 2,
                    ..areas.content
                },
                state.selection,
                state.focus,
                state.draft,
                look,
            );
            if state.focus == Some(Focus::Bars) {
                state.set_focus(Focus::Each);
            }
        }
    }
    strip::paint_left(
        frame.buffer_mut(),
        areas.footer,
        Line::styled(
            "←→ choose · Enter apply/details · Tab fields · Esc close",
            look.role(Role::Muted),
        ),
    );
    if app.usage_controls_active() {
        app.frame_clocks
            .borrow_mut()
            .push(crate::board::view::ClockSurface::UsageChart);
    }
}
fn paint_chart(
    frame: &mut Frame,
    app: &App,
    state: &mut State,
    area: Rect,
    look: crate::look::Look,
    key: Option<String>,
    plot_height: u8,
) -> u16 {
    let data = Rc::clone(state.observed.as_ref().unwrap());
    let active = state.focus.filter(|_| app.focused() == Pane::Rows);
    let status = if app.view.as_ref().is_some_and(|view| view.history_pending) {
        "Updating usage…"
    } else {
        "unique identities"
    };
    let title = format!(
        "TOK / {}m · {status}{}",
        state.selection.each_minutes,
        key.map_or(String::new(), |key| format!(" · {key} controls"))
    );
    strip::paint_left(
        frame.buffer_mut(),
        Rect::new(area.x, area.y, area.width, 1),
        Line::styled(title, look.role(Role::Muted)),
    );
    let control_rows = controls::height(area.width);
    let controls_area = Rect::new(area.x, area.y + 1, area.width, control_rows);
    let controls = controls::paint(
        frame.buffer_mut(),
        controls_area,
        state.selection,
        active,
        state.draft,
        look,
    );
    state.hits.extend(controls);
    let mut plot = Rect::new(area.x, area.y + 1 + control_rows, area.width, 0);
    let display = project::display(&data, state.selection).expect("validated interval projection");
    if state.scene.as_ref().is_none_or(|(old, width, height, _)| {
        old != &display || *width != plot.width || *height != plot_height
    }) {
        let scene = Scene::new(&display, plot.width, plot_height).expect("bounded chart scene");
        state.scene = Some((display, plot.width, plot_height, scene));
    }
    // Scene solves/paints the actual sub-rectangle. Copy its bounded cells into
    // the existing Frame buffer; Ratatui retains the sole terminal diff owner.
    plot.height = state.scene.as_ref().unwrap().3.rows();
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
            if hit.target == Target::Bar(state.bar)
                && hit.area.y == plot.y + u16::from(plot_height) - 1
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
        || super::super::format::range_label(&data, state.selection.range_minutes),
        |index| project::details(&data.buckets[index]).join(" · "),
    );
    strip::paint_left(
        frame.buffer_mut(),
        Rect::new(area.x, plot.bottom(), area.width, 1),
        Line::styled(label, look.role(Role::Muted)),
    );
    plot.bottom() + 1 - area.y
}
