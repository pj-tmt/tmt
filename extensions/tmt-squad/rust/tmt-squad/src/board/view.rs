//! Frame orchestration; surface painters retain their existing owners.

mod detail;
mod footer;
mod header;
pub(in crate::board) mod member_list;
mod notes;
mod overlays;
mod panes;
mod replies;
pub(in crate::board) mod row_paint;
mod rows;
pub(in crate::board) mod scene;
mod tabs;
pub(in crate::board) mod waiting;

use super::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
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
    let home_usage = (!app.loading())
        .then(|| app.home_header_usage(std::time::Instant::now()))
        .flatten()
        .zip(app.view.as_ref())
        .and_then(|(usage, _view)| {
            super::home::usage_of(app, &usage, frame.area().width, app.look())
        });
    render_frame(frame, app, home_usage);
    app.home_counters.borrow_mut().finish(
        app.input_band.get(),
        app.help
            || app.settings.is_some()
            || app.menu.is_some()
            || app.switcher.is_some()
            || app.cron_list.is_some()
            || app.view_picker.is_some()
            || app.theme_picker.is_some(),
    );
}

/// Frame geometry consumes an already formatted header, independent of acquisition.
pub(in crate::board) fn render_frame(
    frame: &mut Frame,
    app: &App,
    home_usage: Option<ratatui::text::Line<'static>>,
) {
    let look = app.look();
    app.input_band.set(None);
    app.hits.borrow_mut().clear();
    app.note_hits.borrow_mut().clear();
    app.link_hits.borrow_mut().clear();
    app.row_starts.borrow_mut().clear();
    app.tab_hits.borrow_mut().clear();
    app.unpicked_hit.set(None);
    app.tabs_overflow.set(false);
    app.title_hits.borrow_mut().clear();
    app.jobs_area.set(ratatui::layout::Rect::default());
    app.scrolls.begin_frame();
    let home_history_pending = !app.loading()
        && app
            .view
            .as_ref()
            .is_some_and(|view| view.home.is_some() && view.history_pending);
    let [tabs, summary, meter_status, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(u16::from(
            header::meter_enabled(app) || home_usage.is_some() || home_history_pending,
        )),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    strip::paint_left(frame.buffer_mut(), tabs, tabs::paint(app, tabs));
    let summary_text = app
        .view
        .as_ref()
        .filter(|view| !app.loading() && view.home.is_some())
        .map_or_else(
            || header::summary_line(app),
            |view| super::home::summary_of(view, summary.width, look),
        );
    let summary_area =
        header::meter_region(app, summary).map_or(summary, |(meter, _)| ratatui::layout::Rect {
            width: meter.x.saturating_sub(summary.x).saturating_sub(2),
            ..summary
        });
    strip::paint_left(frame.buffer_mut(), summary_area, summary_text);
    header::render_meter(frame, app, summary);
    header::render_meter_status(frame, app, meter_status);
    if home_history_pending {
        strip::paint_left(
            frame.buffer_mut(),
            meter_status,
            ratatui::text::Line::styled("Updating usage…", look.role(tmt_cli_style::Role::Muted)),
        );
    } else if let Some(line) = home_usage {
        app.home_counters.borrow_mut().place_header(meter_status);
        strip::paint_left(frame.buffer_mut(), meter_status, line);
    }
    panes::render_body(frame, app, body);
    waiting::inline_prompt(frame, app, body);
    footer::render(frame, app, footer, look);
    overlays::render(frame, app, body, look);
    waiting::prompt(frame, app, body);
    look.selected_words(frame.buffer_mut());
}

#[cfg(test)]
mod tests;
