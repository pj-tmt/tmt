//! A selection of the conversation transcript that follows its text. Its ends are
//! transcript lines and cell columns, not screen cells, so repainting, scrolling,
//! new messages and a changing input band never move or drop it. The passage stays
//! the literal displayed text, never evidence for a request or recipient.
use super::super::{ConversationKey, Rendered, RowTarget};
use ratatui::{crossterm::event::KeyEvent, layout::Rect};
use std::{cmp::Reverse, collections::BTreeMap, ops::Range};
use tmt_tui::components::Editor;

/// One end of a selection. `identity` is the transcript line's text when the end
/// was set; a rebuilt transcript is searched for that line again.
#[derive(Clone)]
struct Spot {
    line: usize,
    column: usize,
    identity: String,
}

impl Spot {
    fn at(rendered: &Rendered, line: usize, column: usize) -> Self {
        Self {
            line,
            column,
            identity: rendered
                .anchor_text
                .line(line)
                .unwrap_or_default()
                .to_owned(),
        }
    }
    fn position(&self) -> (usize, usize) {
        (self.line, self.column)
    }
}

pub(super) enum Resolution {
    /// Both ends sit on their text, so the selection can be painted.
    Attached,
    /// The text changed or was re-wrapped. The frozen passage stays quotable.
    Detached,
    /// The selected lines no longer exist.
    Gone,
}

pub(in crate::board::conversation) struct Anchored {
    pub(super) target: RowTarget,
    pub(super) key: ConversationKey,
    /// Cell width of the displayed rows, and the wrap width of the transcript
    /// those columns count in.
    width: u16,
    wrap: usize,
    fixed: Spot,
    moving: Spot,
    /// The last displayed text of every line between the ends. Lines scrolled
    /// past between two frames fall back to the transcript's own text.
    rows: BTreeMap<usize, String>,
    pub(super) dragging: bool,
    pub(super) control: Option<Rect>,
}

/// The glyph-aligned byte range of `row` between two cell columns.
fn glyphs(row: &str, width: u16, from: usize, to: usize) -> (Editor, Range<usize>) {
    let mut editor = Editor::from(row.to_owned());
    editor.place(width, 0, from, false);
    editor.place(width, 0, to, true);
    let range = editor.selection();
    (editor, range)
}

impl Anchored {
    pub(super) fn start(
        target: RowTarget,
        key: ConversationKey,
        width: u16,
        rendered: &Rendered,
        at: (usize, usize),
        row: &str,
        dragging: bool,
    ) -> Self {
        let spot = Spot::at(rendered, at.0, at.1);
        Self {
            target,
            key,
            width,
            wrap: rendered.width,
            fixed: spot.clone(),
            moving: spot,
            rows: BTreeMap::from([(at.0, row.trim_end().to_owned())]),
            dragging,
            control: None,
        }
    }

    fn bounds(&self) -> (&Spot, &Spot) {
        if self.fixed.position() <= self.moving.position() {
            (&self.fixed, &self.moving)
        } else {
            (&self.moving, &self.fixed)
        }
    }

    /// The first and last transcript line the selection touches.
    pub(super) fn lines(&self) -> (usize, usize) {
        let (first, last) = self.bounds();
        (first.line, last.line)
    }

    /// Whether the ends differ; a bare press selects nothing.
    /// Whether the transcript still wraps as the columns were counted.
    pub(super) fn attached_to(&self, rendered: &Rendered) -> bool {
        rendered.width == self.wrap
    }

    pub(super) fn has_range(&self) -> bool {
        self.fixed.position() != self.moving.position()
    }

    fn columns(&self, line: usize) -> (usize, usize) {
        let (first, last) = self.bounds();
        (
            if line == first.line { first.column } else { 0 },
            if line == last.line {
                last.column
            } else {
                usize::from(self.width)
            },
        )
    }

    /// The `(column, width)` of every selected glyph of the displayed `row`.
    pub(super) fn highlights(&self, line: usize, row: &str) -> Vec<(usize, usize)> {
        let (from, to) = self.columns(line);
        let (editor, range) = glyphs(row, self.width, from, to);
        if range.is_empty() {
            return Vec::new();
        }
        editor.layout(self.width).lines[0]
            .cells
            .iter()
            .filter(|cell| cell.source.start < range.end && cell.source.end > range.start)
            .map(|cell| (cell.column, cell.width))
            .collect()
    }

    pub(super) fn selected(&self) -> Option<String> {
        let (first, last) = self.lines();
        let parts: Vec<_> = (first..=last)
            .map(|line| {
                let row = self.rows.get(&line).map_or("", String::as_str);
                let (from, to) = self.columns(line);
                row[glyphs(row, self.width, from, to).1].to_owned()
            })
            .collect();
        parts
            .iter()
            .any(|part| !part.is_empty())
            .then(|| parts.join("\n"))
    }

    /// Record the text a frame displays for a selected line.
    pub(super) fn capture(&mut self, line: usize, row: &str) {
        self.rows.insert(line, row.trim_end().to_owned());
    }

    fn backfill(&mut self, rendered: &Rendered, lines: Range<usize>) {
        for line in lines {
            if let Some(text) = rendered.anchor_text.line(line) {
                self.rows
                    .entry(line)
                    .or_insert_with(|| text.trim_end().to_owned());
            }
        }
    }

    /// Move the free end, as a drag or a key does.
    pub(super) fn extend(&mut self, rendered: &Rendered, line: usize, column: usize) {
        let (low, high) = (self.moving.line.min(line), self.moving.line.max(line));
        self.moving = Spot::at(rendered, line, column);
        self.backfill(rendered, low..high + 1);
        self.control = None;
    }

    /// Keep both ends on their text through a rebuilt transcript: the same line
    /// nearest to where it was, else detached while the lines still exist.
    pub(super) fn resolve(&mut self, rendered: &Rendered) -> Resolution {
        if rendered.width != self.wrap {
            return Resolution::Detached;
        }
        let text = &rendered.anchor_text;
        let mut lines = [self.fixed.line, self.moving.line];
        for (line, spot) in lines.iter_mut().zip([&self.fixed, &self.moving]) {
            if text.line(spot.line) == Some(spot.identity.as_str()) {
                continue;
            }
            match (0..text.line_count())
                .filter(|index| text.line(*index) == Some(spot.identity.as_str()))
                .min_by_key(|index| (index.abs_diff(spot.line), Reverse(*index)))
            {
                Some(found) => *line = found,
                None if spot.line >= text.line_count() => return Resolution::Gone,
                None => return Resolution::Detached,
            }
        }
        if lines != [self.fixed.line, self.moving.line] {
            self.fixed.line = lines[0];
            self.moving.line = lines[1];
            self.rows.clear();
            let (first, last) = self.lines();
            self.backfill(rendered, first..last + 1);
        }
        Resolution::Attached
    }

    /// Extend by a Shift+arrow key over the lines the frame displays
    /// (`painted` starts at transcript line `offset`) and the selection itself.
    pub(super) fn key(
        &mut self,
        event: KeyEvent,
        rendered: &Rendered,
        offset: usize,
        painted: &[&str],
    ) {
        let (first, last) = self.lines();
        let low = first.min(offset);
        let high = last.max((offset + painted.len()).saturating_sub(1));
        let rows: Vec<_> = (low..=high)
            .map(|line| {
                line.checked_sub(offset)
                    .and_then(|row| painted.get(row).copied())
                    .or_else(|| self.rows.get(&line).map(String::as_str))
                    .or_else(|| rendered.anchor_text.line(line))
                    .unwrap_or_default()
                    .trim_end()
                    .to_owned()
            })
            .collect();
        let mut editor = Editor::from(rows.join("\n"));
        editor.place(self.width, self.fixed.line - low, self.fixed.column, false);
        editor.place(self.width, self.moving.line - low, self.moving.column, true);
        editor.key(event, self.width, usize::MAX);
        let (row, column) = editor.layout(self.width).cursor;
        for (index, text) in rows.into_iter().enumerate() {
            self.rows.insert(low + index, text);
        }
        self.moving = Spot::at(rendered, low + row, column);
        self.control = None;
    }
}
