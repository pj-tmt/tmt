//! Frame orchestration; surface painters retain their existing owners.

mod detail;
mod footer;
mod header;
mod notes;
mod overlays;
mod panes;
mod replies;
pub(in crate::board) mod row_paint;
mod rows;
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
    let look = app.look();
    app.input_band.set(None);
    app.hits.borrow_mut().clear();
    app.note_hits.borrow_mut().clear();
    app.link_hits.borrow_mut().clear();
    app.row_starts.borrow_mut().clear();
    app.tab_hits.borrow_mut().clear();
    app.unpicked_hit.set(None);
    app.title_hits.borrow_mut().clear();
    app.jobs_area.set(ratatui::layout::Rect::default());
    app.scrolls.begin_frame();
    let [tabs, summary, meter_status, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(u16::from(header::meter_enabled(app))),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    strip::paint_left(
        frame.buffer_mut(),
        tabs,
        tabs::paint(app, tabs),
        &look.theme,
        look.depth,
    );
    let summary_text = app
        .view
        .as_ref()
        .filter(|_| !app.loading())
        .and_then(|view| view.home.as_ref())
        .map_or_else(
            || header::summary_line(app),
            |home| super::home::summary(home, summary.width, look),
        );
    let summary_area =
        header::meter_region(app, summary).map_or(summary, |(meter, _)| ratatui::layout::Rect {
            width: meter.x.saturating_sub(summary.x).saturating_sub(2),
            ..summary
        });
    strip::paint_left(
        frame.buffer_mut(),
        summary_area,
        summary_text,
        &look.theme,
        look.depth,
    );
    header::render_meter(frame, app, summary);
    header::render_meter_status(frame, app, meter_status);
    panes::render_body(frame, app, body);
    waiting::inline_prompt(frame, app, body);
    footer::render(frame, app, footer, look);
    overlays::render(frame, app, body, look);
    waiting::prompt(frame, app, body);
}

#[cfg(test)]
mod tests;
