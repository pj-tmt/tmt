//! The Overview primary body has one scroll/selection owner; Usage precedes it.
pub(in crate::board) mod lower;
pub(in crate::board) mod measure;
mod render;
pub(in crate::board) mod wide;
pub(in crate::board) use render::render;
#[cfg(test)]
mod tests;

/// Space needed to retain every currently configured section while Usage grows.
pub(in crate::board) fn minimum_height(app: &crate::board::app::App, width: u16) -> u16 {
    if width >= 140 && app.overview_panels().side_shown() {
        wide::minimum_height(app)
    } else {
        5 + lower::Band::new(app, width, width).map_or(0, |band| band.height as u16 + 1)
    }
}
