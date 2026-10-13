//! Disposable surface derivations retained by one immutable view. New snapshots
//! own a new cache; workspace rows have no legacy grid or boxed-list cache.
use ratatui::text::Line;

#[derive(Default)]
pub(super) struct Derived {
    pub notes: Option<NotebookLines>,
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
