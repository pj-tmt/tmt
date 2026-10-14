//! The three-tab workspace shell, including startup before the roster arrives.

use crate::board::app::App;
use ratatui::{layout::Rect, text::Line};

pub(super) fn paint(app: &App, area: Rect) -> Line<'static> {
    crate::board::workspace::render::tabs(app, area)
}
