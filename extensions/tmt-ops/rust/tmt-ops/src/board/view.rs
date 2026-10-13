//! Frame orchestration; surface painters retain their existing owners.

mod cached;
mod detail;
pub(super) mod footer;
mod header;
pub(in crate::board) mod member_list;
mod notes;
mod overlays;
mod panes;
pub(in crate::board) mod row_paint;
mod rows;
pub(in crate::board) mod scene;
pub(in crate::board) mod stable;
mod tabs;
pub(in crate::board) mod waiting;

use super::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
};
use tmt_cli_style::grid::Align;
use tmt_tui::components::strip;

pub(super) use footer::toggle_label;
#[cfg(test)]
pub(super) use header::SPINNER_DELAY;
pub(super) use header::{spinner_frame, spinner_wait, time_marks};
pub(super) use notes::notebook_lines;

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
    app.home_counters.borrow_mut().begin();
    let global =
        app.workspace_snapshot().is_none() || app.shell.tab == super::shell::OpsTab::Overview;
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
            || app.switcher.is_some()
            || app.cron_list.is_some()
            || app.view_picker.is_some()
            || app.theme_picker.is_some()
            || app.workspace_form.is_some(),
    );
}

/// Resolve transient folds before the existing manual-fold focus reconciliation.
pub(super) fn prepare(app: &mut App, area: Rect) {
    if app.cached_display().is_none() && !app.view.as_ref().is_some_and(|view| view.home.is_some())
    {
        let body = bands(app, area, false, 0)[5];
        panes::prepare(app, body);
    }
    app.set_body_width(area.width);
}

fn bands(app: &App, area: Rect, has_usage: bool, limit_rows: u16) -> [Rect; 7] {
    let home_history_pending = !app.loading()
        && (app.workspace_snapshot().is_none() || app.shell.tab == super::shell::OpsTab::Overview)
        && app
            .view
            .as_ref()
            .is_some_and(|view| view.home.is_some() && view.history_pending);
    let overview = !app.loading()
        && app.workspace_snapshot().is_none()
        && app
            .home_entries()
            .iter()
            .any(|entry| entry.target.section == super::home::USAGE);
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(
            app.view.as_ref().is_some_and(|view| view.home.is_some()),
        )),
        Constraint::Length(1),
        Constraint::Length(limit_rows),
        Constraint::Length(u16::from(
            header::meter_enabled(app) || has_usage || home_history_pending || overview,
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
    let look = app.look();
    app.input_band.set(None);
    app.input_viewport.set(None);
    app.conversation.borrow_mut().pane = None;
    app.stable
        .borrow_mut()
        .begin(app.shown_tab().unwrap_or_default(), frame.area().width);
    app.hits.borrow_mut().clear();
    app.detail_more_hits.borrow_mut().clear();
    app.chip_hits.borrow_mut().clear();
    app.workspace_action_hits.borrow_mut().clear();
    app.note_hits.borrow_mut().clear();
    app.link_hits.borrow_mut().clear();
    app.row_starts.borrow_mut().clear();
    app.tab_hits.borrow_mut().clear();
    app.unpicked_hit.set(None);
    app.tabs_overflow.set(false);
    app.title_hits.borrow_mut().clear();
    app.jobs_area.set(ratatui::layout::Rect::default());
    app.scrolls.begin_frame();
    let overview = (!app.loading() && app.workspace_snapshot().is_none())
        .then(|| {
            app.home_entries()
                .iter()
                .position(|entry| entry.target.section == super::home::USAGE)
        })
        .flatten();
    let home_history_pending = !app.loading()
        && (app.workspace_snapshot().is_none() || app.shell.tab == super::shell::OpsTab::Overview)
        && app
            .view
            .as_ref()
            .is_some_and(|view| view.home.is_some() && view.history_pending);
    let [tabs, boundary, summary, limits, meter_status, body, footer] = bands(
        app,
        frame.area(),
        home_usage.is_some(),
        home_limits.len().min(usize::from(u16::MAX)) as u16,
    );
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
    match app.cached_display() {
        Some(display) => cached::status(frame, app, display, summary_area),
        None => strip::paint_left(frame.buffer_mut(), summary_area, summary_text),
    }
    header::render_meter(frame, app, summary);
    header::render_meter_status(frame, app, meter_status);
    let usage = if home_history_pending {
        Some(ratatui::text::Line::styled(
            "  Updating usage…",
            look.role(tmt_cli_style::Role::Muted),
        ))
    } else {
        home_usage.or_else(|| {
            overview.map(|_| {
                ratatui::text::Line::styled(
                    "  Usage no reading",
                    look.role(tmt_cli_style::Role::Muted),
                )
            })
        })
    };
    if let Some(mut line) = usage {
        app.home_counters.borrow_mut().place_header(meter_status);
        if overview == Some(app.selected) {
            // Reserve the same two-cell cursor slot in every usage state; the
            // text stays muted and the provider tracks never look selected.
            if let Some(first) = line.spans.first_mut() {
                first.content = format!("> {}", first.content.trim_start()).into();
            }
        }
        strip::paint_left(frame.buffer_mut(), meter_status, line);
    }
    if let Some(row) = overview.filter(|_| meter_status.height > 0) {
        app.hits.borrow_mut().push(super::app::Hit {
            y: meter_status.y,
            x: meter_status.x,
            width: meter_status.width,
            target: super::app::HitTarget::Row(row),
        });
    }
    for (offset, line) in home_limits
        .into_iter()
        .enumerate()
        .take(usize::from(limits.height))
    {
        strip::paint_left(
            frame.buffer_mut(),
            ratatui::layout::Rect::new(limits.x, limits.y + offset as u16, limits.width, 1),
            line,
        );
    }
    match app.cached_display() {
        Some(display) => cached::body(frame, app, display, body),
        None => panes::render_body(frame, app, body),
    }
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
