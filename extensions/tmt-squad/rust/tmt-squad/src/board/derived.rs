//! Disposable derivations retained by one immutable view. Width and search
//! select the row layout; new snapshots own a new cache.
use ratatui::text::Line;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Derived {
    pub notes: Option<NotebookLines>,
    pub replies: Option<ReplyBodies>,
    pub grid: Option<Grid>,
    pub composition: Option<super::composition::Cache>,
    /// HOME's painted sections, each held with the inputs it was painted for.
    pub home: super::home::Scenes,
}

pub(super) struct NotebookLines {
    pub width: usize,
    pub look: crate::look::Look,
    pub lines: Vec<Line<'static>>,
    pub sources: Vec<usize>,
    pub links: Vec<super::markdown::Link>,
    pub hits: Vec<super::markdown::LinkHit>,
}

pub(super) struct Grid {
    pub width: usize,
    pub search: String,
    /// Clock-derived row labels: a changed label rebuilds the layout and scene.
    pub extras: Vec<super::view::row_paint::Extra>,
    pub scene: super::view::row_paint::RowPaint,
    #[cfg(test)]
    pub layout: crate::markup::Grid,
    #[cfg(test)]
    pub cells: Vec<tmt_tui::binding::Node>,
}

/// Rendered bodies belong to the immutable view; headers and prompts stay fresh.
pub(super) struct ReplyBodies {
    pub width: usize,
    pub look: crate::look::Look,
    pub bodies: BTreeMap<String, Vec<Line<'static>>>,
}
