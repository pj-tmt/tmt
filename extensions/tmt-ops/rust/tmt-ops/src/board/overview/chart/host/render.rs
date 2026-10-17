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
fn display(app: &App, data: &Series, selection: Selection) -> super::super::model::Series {
    let mut view = project::display(data, selection).expect("validated interval projection");
    if app.shell.tab == OpsTab::Overview && quotas_visible(app) {
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
    usize::from(width)
        >= crate::board::spark_row::minimum_width(&project::spark_rows(&display(
            app, data, selection,
        )))
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
    if app.shell.tab == OpsTab::Overview
        && app.view.as_ref().is_some_and(|view| view.history_pending)
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
    refresh(app, &mut state, now);
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
    if app.shell.tab == OpsTab::Overview {
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
        && !app.usage_controls
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
    if (positive || app.shell.tab == OpsTab::Overview)
        && area.width >= 40
        && fits_chart(app, &data, state.selection, area.width)
        && !app.usage_controls
        && area.height >= chart_rows(app, &data, state.selection)
    {
        if !positive && state.focus == Some(Focus::Bars) {
            state.release_focus();
        }
        paint_chart(frame, app, &mut state, area, look, now);
    } else {
        if !app.usage_controls {
            state.release_focus();
        }
        let title = title(app, &data, state.selection);
        state.hits = controls::header(
            frame.buffer_mut(),
            area,
            state.selection,
            &title,
            app,
            (None, 0),
            false,
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
pub(in crate::board) fn render_controls(frame: &mut Frame, app: &App, body: Rect) {
    let look = app.look();
    let quota_controls = app.quota_history_visible();
    let modal = Modal {
        title: if quota_controls {
            "Usage · Each / Range / quota"
        } else {
            "Usage · Each / Range"
        }
        .into(),
        placement: Placement::Body,
    };
    let mut areas = modal.areas(body, [body.width, body.height], true, false);
    // This modal has no position indicator. At short heights its reserved row
    // belongs to the controls before the optional keyboard reminder.
    let control_rows = controls::height(areas.content.width, app.usage_chart.borrow().selection)
        + u16::from(quota_controls);
    if areas.content.height < control_rows {
        areas.content.height += areas.position.height;
        areas.position.height = 0;
    }
    if areas.content.height < control_rows {
        areas.content.height += areas.footer.height;
        areas.footer.height = 0;
    }
    modal.paint_flat(areas, frame.buffer_mut(), &look.theme, look.depth);
    let mut state = app.usage_chart.borrow_mut();
    state.hits.clear();
    state.visible = false;
    let now = crate::status::now_ms();
    refresh(app, &mut state, now);
    let control_rows =
        controls::height(areas.content.width, state.selection) + u16::from(quota_controls);
    if areas.content.height >= control_rows {
        state.visible = true;
        if state.focus.is_none() {
            state.set_focus(Focus::Each);
        }
        if areas.content.width >= 80
            && controls::header_fits(
                areas.content.width,
                state.selection,
                &title(app, state.observed.as_ref().unwrap(), state.selection),
            )
            && fits_chart(
                app,
                state.observed.as_ref().unwrap(),
                state.selection,
                areas.content.width,
            )
            && areas.content.height
                >= chart_rows(app, state.observed.as_ref().unwrap(), state.selection)
        {
            paint_chart(frame, app, &mut state, areas.content, look, now);
        } else {
            state.hits = controls::paint(
                frame.buffer_mut(),
                Rect {
                    height: control_rows - u16::from(quota_controls),
                    ..areas.content
                },
                state.selection,
                state.focus,
                state.draft,
                app,
            );
            if quota_controls {
                let hits = controls::quota(
                    frame.buffer_mut(),
                    Rect::new(
                        areas.content.x,
                        areas.content.y + control_rows - 1,
                        areas.content.width,
                        1,
                    ),
                    state.quota_window,
                    state.focus,
                    state.draft,
                    app,
                );
                state.hits.extend(hits);
            }
            if state.focus == Some(Focus::Bars) {
                state.set_focus(Focus::Each);
            }
        }
    }
    if state.focus == Some(Focus::Quota)
        && !state
            .hits
            .iter()
            .any(|hit| matches!(hit.target, Target::Quota(_)))
    {
        state.set_focus(Focus::Each);
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
    now: u64,
) -> u16 {
    let data = Rc::clone(state.observed.as_ref().unwrap());
    let active = state.focus.filter(|_| app.focused() == Pane::Rows);
    let title = title(app, &data, state.selection);
    state.hits.extend(controls::header(
        frame.buffer_mut(),
        area,
        state.selection,
        &title,
        app,
        (active, state.draft),
        app.usage_controls
            || data
                .buckets
                .iter()
                .any(|bucket| bucket.tokens.is_some_and(|tokens| tokens > 0)),
    ));
    let view = display(app, &data, state.selection);
    let drivers: Vec<_> = view.drivers.iter().cloned().collect();
    let rows = project::spark_rows(&view);
    let quota_width = if app.shell.tab == OpsTab::Overview && quotas_visible(app) {
        crate::board::spark_row::quota_width(area.width, &rows)
    } else {
        0
    };
    if state.scene.as_ref().is_none_or(|(old, width, scene)| {
        old != &rows || *width != area.width || scene.quota_width() != quota_width
    }) {
        let scene = Scene::new(&rows, area.width, quota_width).expect("bounded shared spark rows");
        state.scene = Some((rows, area.width, scene));
    }
    let scene = &state.scene.as_ref().unwrap().2;
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
    let inspection = state
        .hover
        .or(active
            .filter(|focus| *focus == Focus::Bars)
            .map(|_| state.bar))
        .map(|index| project::details(&data.buckets[index]).join(" · "));
    let hits = scene
        .paint(
            &mut buffer,
            &look.theme,
            look.depth,
            &quotas,
            inspection.as_deref(),
        )
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
        if !data
            .buckets
            .iter()
            .any(|bucket| bucket.tokens.is_some_and(|tokens| tokens > 0))
        {
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
            target: Target::Bar(suffix.parse().ok()?),
        })
    }));
    if active == Some(Focus::Bars) {
        for hit in &state.hits {
            if hit.target == Target::Bar(state.bar) {
                buffer.set_style(hit.area, look.selection());
            }
        }
    }
    for y in plot.y..plot.bottom() {
        for x in plot.x..plot.right() {
            frame.buffer_mut()[(x, y)] = buffer[(x, y)].clone();
        }
    }
    plot.bottom() - area.y
}
