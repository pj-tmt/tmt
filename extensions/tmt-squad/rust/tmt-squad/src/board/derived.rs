//! Disposable derivations retained by one immutable view. Width and search
//! select the row layout; new snapshots own a new cache.
use ratatui::text::Line;

#[derive(Default)]
pub(super) struct Derived {
    pub notes: Option<(usize, Vec<Line<'static>>)>,
    pub grid: Option<Grid>,
}

pub(super) struct Grid {
    pub width: usize,
    pub search: String,
    pub widths: Vec<Option<usize>>,
}
