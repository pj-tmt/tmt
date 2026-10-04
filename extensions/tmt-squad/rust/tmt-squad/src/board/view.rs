//! Frame orchestration; surface painters retain their existing owners.

mod detail;
mod footer;
mod header;
mod notes;
mod overlays;
mod panes;
mod replies;
mod rows;
mod tabs;

use super::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    widgets::Paragraph,
};
use tmt_cli_style::grid::Align;

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
    app.hits.borrow_mut().clear();
    app.note_hits.borrow_mut().clear();
    app.link_hits.borrow_mut().clear();
    app.row_starts.borrow_mut().clear();
    app.tab_hits.borrow_mut().clear();
    app.title_hits.borrow_mut().clear();
    app.scrolls.begin_frame();
    let [tabs, summary, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(Paragraph::new(tabs::tab_line(app, tabs)), tabs);
    let summary_text = app
        .view
        .as_ref()
        .filter(|_| !app.loading())
        .and_then(|view| view.home.as_ref())
        .map_or_else(
            || header::summary_line(app),
            |home| super::home::summary(home, summary.width, look),
        );
    frame.render_widget(Paragraph::new(summary_text), summary);
    header::render_meter(frame, app, summary);
    panes::render_body(frame, app, body);
    footer::render(frame, app, footer, look);
    overlays::render(frame, app, body, look);
}

#[cfg(test)]
mod tests;
