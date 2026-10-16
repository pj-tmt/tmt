//! The Overview body composes the selectable list and its narrow lower band.
use ratatui::layout::Rect;
pub(in crate::board) fn render(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    area: Rect,
) {
    super::super::view::render(frame, app, area);
}
