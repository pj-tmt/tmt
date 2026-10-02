//! Disposable derivations retained by one immutable view. Width and search
//! select the row layout; new snapshots own a new cache.
use ratatui::text::Line;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Derived {
    pub notes: Option<(usize, crate::look::Look, Vec<Line<'static>>)>,
    pub replies: Option<ReplyBodies>,
    pub grid: Option<Grid>,
}

pub(super) struct Grid {
    pub width: usize,
    pub search: String,
    pub widths: Vec<Option<usize>>,
}

/// Rendered bodies belong to the immutable view; headers and prompts stay fresh.
pub(super) struct ReplyBodies {
    pub width: usize,
    pub look: crate::look::Look,
    pub bodies: BTreeMap<String, Vec<Line<'static>>>,
}
