//! The full Overview body consumes shared Squads/Waiting modules once.
use ratatui::layout::Rect;
pub(in crate::board) fn render(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    area: Rect,
) {
    if app
        .overview_panels()
        .shown(crate::config::OverviewPanel::Monitor)
    {
        super::super::view::render(frame, app, area);
    }
}
