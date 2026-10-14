//! Frame orchestration; surface painters retain their existing owners.

pub(super) mod footer;
mod header;
mod overlays;
mod panes;
pub(in crate::board) mod scene;
mod tabs;
pub(in crate::board) mod waiting;

use super::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
};
use tmt_cli_style::grid::Align;
use tmt_tui::components::strip;

pub(in crate::board) use header::ClockSurface;
#[cfg(test)]
pub(super) use header::SPINNER_DELAY;
pub(super) use header::{spinner_frame, spinner_wait, time_marks};

/// Exactly `width` display cells: truncated with an ellipsis, or padded.
pub fn fit(text: &str, width: usize) -> String {
    tmt_tui::text::fit_line(
        text,
        width.min(usize::from(u16::MAX)) as u16,
        tmt_tui::style::TextFlow::Truncate,
        Align::Left,
    )
}

pub fn render(frame: &mut Frame, app: &App) {
    let _rows = app.workspace_frame_rows();
    app.home_counters.borrow_mut().begin();
    // Overview owns its Subscriptions panel; do not build or register the former strip too.
    let global = app.workspace_snapshot().is_none();
    let home_usage = (!app.loading() && global)
        .then(|| app.home_header_usage(std::time::Instant::now()))
        .flatten()
        .zip(app.view.as_ref())
        .and_then(|(usage, _view)| {
            super::home::usage_of(app, &usage, frame.area().width, app.look())
        });
    let home_limits = if app.loading() || !global {
        Vec::new()
    } else {
        super::home::limits_of(app, frame.area().width, app.look())
    };
    render_frame(frame, app, home_usage, home_limits);
    app.home_counters.borrow_mut().finish(
        app.input_band.get(),
        app.help
            || app.settings.is_some()
            || app.menu.is_some()
            || app.cron_list.is_some()
            || app.theme_picker.is_some()
            || app.workspace_form.as_ref().is_some_and(|draft| draft.shown),
    );
}

/// Record the frame width before reconciling responsive workspace focus.
pub(super) fn prepare(app: &mut App, area: Rect) {
    app.set_body_width(area.width);
}

fn bands(app: &App, area: Rect, has_usage: bool, limit_rows: u16) -> [Rect; 7] {
    let home_history_pending = !app.loading()
        && (app.workspace_snapshot().is_none() || app.shell.tab == super::shell::OpsTab::Overview)
        && app
            .view
            .as_ref()
            .is_some_and(|view| view.home.is_some() && view.history_pending);
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(
            app.view.as_ref().is_some_and(|view| view.home.is_some()),
        )),
        Constraint::Length(1),
        Constraint::Length(limit_rows),
        Constraint::Length(u16::from(
            header::meter_enabled(app) || has_usage || home_history_pending,
        )),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area)
}

/// Frame geometry consumes an already formatted header, independent of acquisition.
pub(in crate::board) fn render_frame(
    frame: &mut Frame,
    app: &App,
    home_usage: Option<ratatui::text::Line<'static>>,
    home_limits: Vec<ratatui::text::Line<'static>>,
) {
    let _rows = app.workspace_frame_rows();
    let look = app.look();
    app.input_band.set(None);
    app.input_viewport.set(None);
    app.conversation.borrow_mut().pane = None;
    app.hits.borrow_mut().clear();
    app.frame_clocks.borrow_mut().clear();
    app.chip_hits.borrow_mut().clear();
    app.workspace_action_hits.borrow_mut().clear();
    app.overview_sort_hits.borrow_mut().clear();
    app.cron_scope_hits.borrow_mut().clear();
    app.row_starts.borrow_mut().clear();
    app.tab_hits.borrow_mut().clear();
    app.scrolls.begin_frame();
    let home_history_pending = !app.loading()
        && (app.workspace_snapshot().is_none() || app.shell.tab == super::shell::OpsTab::Overview)
        && app
            .view
            .as_ref()
            .is_some_and(|view| view.home.is_some() && view.history_pending);
    let overview =
        app.workspace_snapshot().is_some() && app.shell.tab == super::shell::OpsTab::Overview;
    let [tabs, boundary, summary, limits, meter_status, body, footer] = if overview {
        Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(0),
            Constraint::Length(0),
            Constraint::Length(0),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(frame.area())
    } else {
        bands(
            app,
            frame.area(),
            home_usage.is_some(),
            home_limits.len().min(usize::from(u16::MAX)) as u16,
        )
    };
    app.conversation.borrow_mut().area = body;
    strip::paint_left(frame.buffer_mut(), tabs, tabs::paint(app, tabs));
    if boundary.height > 0 {
        strip::paint_left(
            frame.buffer_mut(),
            boundary,
            ratatui::text::Line::styled(
                scene::rule("", usize::from(boundary.width)),
                look.role(tmt_cli_style::Role::Dim),
            ),
        );
    }
    let summary_text = if app.workspace_snapshot().is_some() {
        super::workspace::render::summary(app, summary.width)
    } else {
        app.view
            .as_ref()
            .filter(|view| !app.loading() && view.home.is_some())
            .map_or_else(
                || header::summary_line(app),
                |view| super::home::summary_of(view, summary.width, look),
            )
    };
    let summary_area =
        header::meter_region(app, summary).map_or(summary, |(meter, _)| ratatui::layout::Rect {
            width: meter.x.saturating_sub(summary.x).saturating_sub(2),
            ..summary
        });
    strip::paint_left(frame.buffer_mut(), summary_area, summary_text);
    header::render_meter(frame, app, summary);
    header::render_meter_status(frame, app, meter_status);
    let usage = if home_history_pending {
        Some(ratatui::text::Line::styled(
            "  Updating usage…",
            look.role(tmt_cli_style::Role::Muted),
        ))
    } else {
        home_usage
    };
    if let Some(line) = usage {
        app.home_counters.borrow_mut().place_header(meter_status);
        strip::paint_left(frame.buffer_mut(), meter_status, line);
    }
    if !home_limits.is_empty() && limits.height > 0 {
        app.frame_clocks.borrow_mut().push(ClockSurface::Limits {
            width: limits.width,
            rows: usize::from(limits.height),
        });
    }
    for (offset, line) in home_limits
        .into_iter()
        .enumerate()
        .take(usize::from(limits.height))
    {
        app.hits.borrow_mut().push(super::app::Hit {
            x: limits.x,
            y: limits.y + offset as u16,
            width: limits.width,
            target: super::app::HitTarget::Quota,
        });
        strip::paint_left(
            frame.buffer_mut(),
            ratatui::layout::Rect::new(limits.x, limits.y + offset as u16, limits.width, 1),
            line,
        );
    }
    panes::render_body(frame, app, body);
    waiting::inline_prompt(frame, app, body);
    footer::render(frame, app, footer, look);
    overlays::render(frame, app, body, look);
    super::workspace::render::dialog(frame, app, body);
    waiting::prompt(frame, app, body);
    super::mentions::render(frame, app);
    look.selected_words(frame.buffer_mut());
    header::finish_meter_styles(frame, app, summary);
}

#[cfg(test)]
mod tests;
