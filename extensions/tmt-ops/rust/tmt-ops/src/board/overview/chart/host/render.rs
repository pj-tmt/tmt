//! Inline and modal placement share one chart painter and semantic scene cache.
use super::*;
use ratatui::{Frame, buffer::Buffer};

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
fn display(app: &App, data: &Series, selection: Selection) -> super::super::model::Series {
    let mut view = project::display(data, selection).expect("validated interval projection");
    if app.shell.tab == OpsTab::Agents && quotas_visible(app) {
        view.drivers.extend(
            app.limits
                .iter()
                .flatten()
                .map(|standing| standing.provider.id().to_owned()),
        );
    }
    view
}
fn fits_chart(app: &App, data: &Series, selection: Selection, width: u16) -> bool {
    crate::board::spark_row::plot_layout(
        width,
        &project::spark_rows(&display(app, data, selection)),
        app.shell.tab == OpsTab::Agents && quotas_visible(app),
    )
    .0 > 0
}
fn resolve(app: &App, state: &mut State, width: u16, now: u64) {
    refresh(app, state, now);
    let rows = project::spark_rows(&display(
        app,
        state.observed.as_ref().unwrap(),
        state.selection,
    ));
    let (plot, _) = crate::board::spark_row::plot_layout(
        width,
        &rows,
        app.shell.tab == OpsTab::Agents && quotas_visible(app),
    );
    state.resolve_width(plot);
    refresh(app, state, now);
}
fn chart_rows(app: &App, data: &Series, selection: Selection) -> u16 {
    1 + Scene::rows_for(&project::spark_rows(&display(app, data, selection)))
}
pub(super) fn quotas_visible(app: &App) -> bool {
    crate::board::home::limits_enabled(app)
        && app
            .overview_panels()
            .shown(crate::config::OverviewPanel::Subscriptions)
        && app.limits.as_ref().is_none_or(|limits| !limits.is_empty())
}
fn title(app: &App, data: &Series, selection: Selection) -> String {
    let title = project::summary(data, selection.range_minutes);
    if app.shell.tab == OpsTab::Agents && app.view.as_ref().is_some_and(|view| view.history_pending)
    {
        format!("{title} · Updating usage…")
    } else {
        title
    }
}

/// Overview's full-width Usage band precedes the existing body panes. This
/// admission uses a fixed target minimum, so pane placement never recurses.
pub(in crate::board) fn overview_demand(app: &App, width: u16, height: u16, now: u64) -> u16 {
    if width == 0 || height <= 7 {
        return 0;
    }
    let mut state = app.usage_chart.borrow_mut();
    resolve(app, &mut state, width, now);
    let selection = state.selection;
    let source = Rc::clone(state.observed.as_ref().expect("refreshed source"));
    let rows = chart_rows(app, &source, selection) + 1; // header breathing row and trailing gap
    if width >= 40 && fits_chart(app, &source, selection, width) && height >= rows + 7 {
        rows
    } else {
        1
    }
}

pub(in crate::board) fn demand(app: &App, width: u16, target_row_budget: u16, now: u64) -> u16 {
    if app.shell.tab == OpsTab::Agents {
        return overview_demand(app, width, target_row_budget, now);
    }
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
    let data = if let Some(data) = state.observed.as_ref().filter(|data| data.grid == grid) {
        Rc::clone(data)
    } else {
        Rc::new(app.usage_series(state.selection, now))
    };
    let used = chart_rows(app, &data, state.selection);
    if data
        .buckets
        .iter()
        .any(|bucket| bucket.tokens.is_some_and(|tokens| tokens > 0))
        && width >= 40
        && fits_chart(app, &data, state.selection, width)
        && target_row_budget >= used
        && list_capacity(app, Rect::new(0, used, width, target_row_budget - used)) >= 6
    {
        return used;
    }
    u16::from(list_capacity(app, Rect::new(0, 1, width, target_row_budget - 1)) > 0)
}
pub(in crate::board) fn render_panel(frame: &mut Frame, app: &App, area: Rect, now: u64) {
    let look = app.look();
    let mut state = app.usage_chart.borrow_mut();
    state.hits.clear();
    state.visible = false;
    if area.is_empty() || app.workspace_snapshot().is_none() {
        state.hover = None;
        return;
    }
    state.visible = true;
    refresh(app, &mut state, now);
    let data = Rc::clone(state.observed.as_ref().unwrap());
    let positive = data
        .buckets
        .iter()
        .any(|bucket| bucket.tokens.is_some_and(|tokens| tokens > 0));
    if (positive || app.shell.tab == OpsTab::Agents)
        && area.width >= 40
        && fits_chart(app, &data, state.selection, area.width)
        && area.height >= chart_rows(app, &data, state.selection)
    {
        if !data.buckets.iter().any(|bucket| bucket.tokens.is_some())
            && state.focus == Some(Focus::Bars)
        {
            state.release_focus();
        }
        paint_chart(frame, app, &mut state, area, look, now);
    } else {
        // Digest opens this chart explicitly. Its empty summary must retain
        // Range/Each focus between u and Enter, just like a populated chart.
        if !app.usage_controls
            && (app.shell.tab != OpsTab::Digest
                || !matches!(state.focus, Some(Focus::Range | Focus::Each)))
        {
            state.release_focus();
        }
        let title = title(app, &data, state.selection);
        state.hits = controls::header(
            frame.buffer_mut(),
            area,
            state.selection,
            &title,
            app,
            None,
            state.each,
        );
    }
    if state.focus == Some(Focus::Quota)
        && !state
            .hits
            .iter()
            .any(|hit| matches!(hit.target, Target::Quota(_)))
    {
        state.release_focus();
    }
    if !app.modal_open() {
        app.frame_clocks
            .borrow_mut()
            .push(crate::board::view::ClockSurface::UsageChart);
    }
}
pub(in crate::board) fn render_controls(frame: &mut Frame, app: &App, _body: Rect) {
    let look = app.look();
    let mut state = app.usage_chart.borrow_mut();
    let State { dropdown, hits, .. } = &mut *state;
    if let Some(dropdown) = dropdown {
        dropdown.reanchor(hits);
        dropdown.render(frame, look, frame.area());
    }
}
fn paint_chart(
    frame: &mut Frame,
    app: &App,
    state: &mut State,
    area: Rect,
    look: crate::look::Look,
    now: u64,
) -> u16 {
    resolve(app, state, area.width, now);
    let data = Rc::clone(state.observed.as_ref().unwrap());
    let active = state.focus.filter(|_| app.focused() == Pane::Rows);
    let selected = state
        .pinned
        .and_then(|from| data.buckets.iter().position(|b| b.from_ms == from))
        .or_else(|| {
            state
                .pinned
                .is_none()
                .then_some(
                    state.hover.or(active
                        .filter(|focus| *focus == Focus::Bars)
                        .map(|_| state.bar)),
                )
                .flatten()
        });
    let title = selected.map_or_else(
        || title(app, &data, state.selection),
        |index| project::readout(&data.buckets[index]),
    );
    state.hits.extend(controls::header(
        frame.buffer_mut(),
        area,
        state.selection,
        &title,
        app,
        active,
        state.each,
    ));
    let view = display(app, &data, state.selection);
    let drivers: Vec<_> = view.drivers.iter().cloned().collect();
    let mut rows = selected.map_or_else(
        || project::spark_rows(&view),
        |index| project::inspected_rows(&view, &data.buckets[index]),
    );
    let (_, quota_width) = crate::board::spark_row::plot_layout(
        area.width,
        &rows,
        app.shell.tab == OpsTab::Agents && quotas_visible(app),
    );
    let scale =
        crate::board::spark_row::scale(rows.rows.iter().flat_map(|row| row.values.iter().copied()))
            .unwrap_or(0);
    let window = state.viewport.window(rows.buckets);
    for row in &mut rows.rows {
        row.values = row.values[window.clone()].to_vec();
    }
    rows.buckets = window.len();
    rows.axis = if state.selection.range_minutes <= 1_440
        && window.start == 0
        && window.end == data.buckets.len()
    {
        super::super::format::axis(state.selection.range_minutes)
    } else {
        let at = |index: usize| {
            if index == data.buckets.len() {
                return "now".into();
            }
            let pattern = if (window.len() as u32 * state.selection.each_minutes) >= 1_440 {
                "%b %-d"
            } else {
                "%H:%M"
            };
            jiff::Timestamp::from_millisecond(data.buckets[index].from_ms)
                .map(|time| time.strftime(pattern).to_string())
                .unwrap_or_else(|_| "—".into())
        };
        [
            at(window.start),
            at((window.start + window.end) / 2),
            at(window.end),
        ]
    };
    if window.start > 0 {
        rows.axis[0].insert_str(0, "  ");
    }
    if window.end < data.buckets.len() {
        rows.axis[2].push_str("  ");
    }
    if let Some(index) = selected.filter(|index| window.contains(index)) {
        let drawn = state.viewport.width;
        let marker = crate::board::spark_row::bar_center(drawn, window.len(), index - window.start);
        let lengths = rows.axis.each_ref().map(|s| s.chars().count());
        let starts = [
            0,
            drawn.saturating_sub(lengths[1]) / 2,
            drawn.saturating_sub(lengths[2]),
        ];
        for (label, (start, length)) in rows.axis.iter_mut().zip(starts.into_iter().zip(lengths)) {
            if (start..start + length).contains(&marker) {
                label.clear();
            }
        }
    }
    if state
        .scene
        .as_ref()
        .is_none_or(|(old, width, old_scale, scene)| {
            old != &rows
                || *width != area.width
                || *old_scale != scale
                || scene.quota_width() != quota_width
        })
    {
        let scene = Scene::with_scale(&rows, area.width, quota_width, scale)
            .expect("bounded shared spark rows");
        state.scene = Some((rows, area.width, scale, scene));
    }
    let scene = &state.scene.as_ref().unwrap().3;
    let plot = Rect::new(area.x, area.y + 1, area.width, scene.rows());
    let quotas: Vec<_> = drivers
        .iter()
        .map(|driver| {
            (quota_width > 0).then(|| {
                crate::board::overview::quota::usage_row(
                    crate::board::overview::quota::projection(app, driver, state.quota_window, now),
                    now,
                    quota_width,
                    look,
                )
            })
        })
        .collect();
    if quota_width > 0 {
        app.frame_clocks
            .borrow_mut()
            .push(crate::board::view::ClockSurface::OverviewUsageQuota { width: quota_width });
    }
    let mut buffer = Buffer::empty(plot);
    let hits = scene
        .paint(&mut buffer, &look.theme, look.depth, &quotas, None)
        .expect("admitted provider rows");
    if quota_width > 0
        && let Some(heading) = hits.iter().find(|hit| {
            hit.id
                .and_then(|path| path.last())
                .is_some_and(|id| id == "heading-quota")
        })
    {
        let rect = Rect::new(
            heading.rect.x as u16 + 3,
            heading.rect.y as u16,
            quota_width,
            1,
        )
        .intersection(plot);
        state.hits.extend(controls::quota(
            &mut buffer,
            rect,
            state.quota_window,
            active,
            state.draft,
            app,
        ));
    }
    for hit in &hits {
        if hit
            .id
            .and_then(|path| path.last())
            .is_some_and(|id| id.starts_with("quota-"))
        {
            app.hits.borrow_mut().push(crate::board::app::Hit {
                x: hit.rect.x as u16,
                y: hit.rect.y as u16,
                width: hit.rect.width as u16,
                target: crate::board::app::HitTarget::Quota,
            });
        }
    }
    state.hits.extend(hits.iter().filter_map(|hit| {
        if !data.buckets.iter().any(|bucket| bucket.tokens.is_some()) {
            return None;
        }
        let (_, suffix) = hit.id?.last()?.rsplit_once("bar-")?;
        Some(Hit {
            area: Rect::new(
                hit.rect.x.try_into().ok()?,
                hit.rect.y.try_into().ok()?,
                hit.rect.width.try_into().ok()?,
                hit.rect.height.try_into().ok()?,
            ),
            target: Target::Bar(window.start + suffix.parse::<usize>().ok()?),
        })
    }));
    // Selection is one neutral column across all provider rows, including the
    // unknown row. Axis markers use this same scene geometry.
    let mut column = None;
    for hit in &state.hits {
        if matches!(hit.target, Target::Bar(index) if Some(index) == selected) {
            let selected = look.selection();
            let fill = selected.bg.map_or_else(
                || {
                    ratatui::style::Style::default()
                        .add_modifier(ratatui::style::Modifier::REVERSED)
                },
                |bg| ratatui::style::Style::default().bg(bg),
            );
            buffer.set_style(hit.area, fill);
            column = Some(hit.area.x + hit.area.width / 2);
        }
    }
    if let Some(axis) = hits.iter().find(|hit| {
        hit.id
            .and_then(|p| p.last())
            .is_some_and(|id| id == "time-ticks")
    }) {
        let y = axis.rect.y as u16;
        let x = axis.rect.x as u16;
        let right = x + axis.rect.width as u16 - 1;
        if let Some(x) = column {
            buffer[(x, y)].set_symbol("▲");
        }
        // Keep an inspected edge bucket's marker. Its history arrow moves into
        // the existing blank axis gutter, without moving the selected column.
        if window.start > 0 {
            let edge = if column == Some(x) { x - 1 } else { x };
            buffer[(edge, y)].set_symbol("◂");
        }
        if window.end < data.buckets.len() {
            let edge = if column == Some(right) {
                right + 1
            } else {
                right
            };
            buffer[(edge, y)].set_symbol("▸");
        }
    }
    for y in plot.y..plot.bottom() {
        for x in plot.x..plot.right() {
            frame.buffer_mut()[(x, y)] = buffer[(x, y)].clone();
        }
    }
    plot.bottom() - area.y
}
