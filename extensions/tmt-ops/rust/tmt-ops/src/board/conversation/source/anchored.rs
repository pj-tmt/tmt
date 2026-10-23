//! A selection of the conversation transcript that follows its text. Its ends are
//! transcript lines and cell columns, not screen cells, so repainting, scrolling,
//! new messages and a changing input band never move or drop it. The passage stays
//! the literal displayed text, never evidence for a request or recipient.
use super::super::{ConversationKey, Rendered, RowTarget};
use ratatui::crossterm::event::KeyEvent;
use std::{cmp::Reverse, collections::BTreeMap, ops::Range};
use tmt_tui::components::Editor;

/// One end of a selection. `identity` is the displayed text of its transcript line
/// when the end was set; a rebuilt transcript is searched for that line again.
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
            identity: displayed_line(rendered, line),
        }
    }
    fn position(&self) -> (usize, usize) {
        (self.line, self.column)
    }
    /// The line of `text` this end stands on now, as `resolve` describes.
    fn follow(&self, lines: &[String], width: u16, gutter: usize) -> Option<usize> {
        let nearest = |keep: &dyn Fn(&str) -> bool| {
            (0..lines.len())
                .filter(|index: &usize| keep(&lines[*index]))
                .min_by_key(|index: &usize| (index.abs_diff(self.line), Reverse(*index)))
        };
        let old = self.identity.as_str();
        if lines.get(self.line).map(String::as_str) == Some(old) {
            return Some(self.line);
        }
        nearest(&|line| line == old)
            .or_else(|| {
                (!old.is_empty())
                    .then(|| nearest(&|line| line.starts_with(old)))
                    .flatten()
            })
            .or_else(|| {
                // Text changed in place (an age stamp, a progress line): the same line, as
                // long as what lies before this end is what it was.
                // Columns count displayed cells; the transcript's text has no gutter.
                let kept = glyphs(old, width, 0, self.column.saturating_sub(gutter))
                    .1
                    .end;
                let now = lines.get(self.line)?;
                (now.get(..kept) == old.get(..kept)).then_some(self.line)
            })
    }
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
    pub(super) control: Option<super::Menu>,
}

/// The transcript's text on displayed `line`, without the gutter.
fn displayed_line(rendered: &Rendered, line: usize) -> String {
    rendered
        .mapped
        .lines
        .get(line)
        .map_or_else(String::new, |line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
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

    /// Whether the transcript still wraps as the columns were counted.
    pub(super) fn attached_to(&self, rendered: &Rendered) -> bool {
        rendered.width == self.wrap
    }

    /// Whether the ends differ; a bare press selects nothing.
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

    /// What each selected line displays inside the selection, with the byte range of that
    /// text in the line's painted row.
    fn parts(&self) -> Vec<(String, Range<usize>)> {
        let (first, last) = self.lines();
        (first..=last)
            .map(|line| {
                let row = self.rows.get(&line).map_or("", String::as_str);
                let (from, to) = self.columns(line);
                let range = glyphs(row, self.width, from, to).1;
                (row[range.clone()].to_owned(), range)
            })
            .collect()
    }

    pub(super) fn selected(&self) -> Option<String> {
        let parts: Vec<_> = self.parts().into_iter().map(|(text, _)| text).collect();
        parts
            .iter()
            .any(|part| !part.is_empty())
            .then(|| parts.join("\n"))
    }

    /// The selection as the source its messages were written as; lines that carry no
    /// source (headers, status rows) keep what they display.
    pub(super) fn source(&self, map: &super::super::copy::Map) -> Option<String> {
        let parts = self.parts();
        let (first, last) = self.lines();
        let shown: Vec<String> = parts.iter().map(|(text, _)| text.clone()).collect();
        shown.iter().any(|part| !part.is_empty()).then(|| {
            super::super::copy::passage(
                map,
                &super::super::copy::Selected {
                    first,
                    last,
                    shown: &shown,
                    start: super::super::copy::End {
                        row: self.rows.get(&first).map_or("", String::as_str),
                        at: parts.first().map_or(0, |(_, range)| range.start),
                    },
                    end: super::super::copy::End {
                        row: self.rows.get(&last).map_or("", String::as_str),
                        at: parts.last().map_or(0, |(_, range)| range.end),
                    },
                },
            )
        })
    }

    /// Record the text a frame displays for a selected line.
    pub(super) fn capture(&mut self, line: usize, row: &str) {
        self.rows.insert(line, row.trim_end().to_owned());
    }

    fn backfill(&mut self, rendered: &Rendered, lines: Range<usize>) {
        for line in lines.filter(|line| *line < rendered.mapped.lines.len()) {
            self.rows
                .entry(line)
                .or_insert_with(|| displayed_line(rendered, line));
        }
    }

    /// Move the free end, as a drag or a key does.
    pub(super) fn extend(&mut self, rendered: &Rendered, line: usize, column: usize) {
        let (low, high) = (self.moving.line.min(line), self.moving.line.max(line));
        self.moving = Spot::at(rendered, line, column);
        self.backfill(rendered, low..high + 1);
        self.control = None;
    }

    /// Keep both ends on their line through a rebuilt transcript. An end follows the
    /// same text; failing that, the line its old text grew into (a streaming agent
    /// appends to the line being read); failing that, the line at the same place whose
    /// text changed in place after the end (an age stamp, a progress line). `false` when the transcript
    /// was re-wrapped or an end's line is gone: the selection can no longer be shown, so
    /// it ends.
    pub(super) fn resolve(&mut self, rendered: &Rendered) -> bool {
        if rendered.width != self.wrap {
            return false;
        }
        let lines: Vec<String> = (0..rendered.mapped.lines.len())
            .map(|line| displayed_line(rendered, line))
            .collect();
        let (Some(fixed), Some(moving)) = (
            self.fixed.follow(&lines, self.width, rendered.gutter),
            self.moving.follow(&lines, self.width, rendered.gutter),
        ) else {
            return false;
        };
        let moved = [fixed, moving] != [self.fixed.line, self.moving.line];
        self.fixed.line = fixed;
        self.moving.line = moving;
        // The ends now stand for what the transcript shows, so the next rebuild
        // compares against it.
        for spot in [&mut self.fixed, &mut self.moving] {
            spot.identity = lines[spot.line].clone();
        }
        if moved {
            self.rows.clear();
            let (first, last) = self.lines();
            self.backfill(rendered, first..last + 1);
        }
        true
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
                    .map_or_else(
                        || displayed_line(rendered, line),
                        |row| row.trim_end().to_owned(),
                    )
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
