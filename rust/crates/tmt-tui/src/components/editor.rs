//! Grapheme editing and display-cell geometry. Applications own drafts, limits,
//! submission and effects; this component never interprets a recipient or key map.
use std::{cell::RefCell, ops::Range, rc::Rc};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tmt_cli_style::table::escape;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug, Default)]
pub struct Editor {
    text: String,
    cursor: usize,
    anchor: Option<usize>,
    preferred_column: Option<usize>,
    folds: Vec<Fold>,
    next_fold: u64,
    geometry: RefCell<Option<(u16, Rc<Vec<EditorLine>>)>>,
}

// Rendering a draft must not change revision equality or send-clear admission.
impl PartialEq for Editor {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.cursor == other.cursor
            && self.anchor == other.anchor
            && self.preferred_column == other.preferred_column
            && self.folds == other.folds
            && self.next_fold == other.next_fold
    }
}
impl Eq for Editor {}

/// An application-labelled source range. Folding never replaces source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    pub id: u64,
    pub source: Range<usize>,
    pub label: String,
    pub expanded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorCell {
    pub text: String,
    pub source: Range<usize>,
    pub column: usize,
    pub width: usize,
}

/// A single geometry projection is used for paint, caret and pointer placement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorLine {
    pub cells: Vec<EditorCell>,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorLayout {
    pub lines: Rc<Vec<EditorLine>>,
    pub cursor: (usize, usize),
}

impl From<String> for Editor {
    fn from(text: String) -> Self {
        Self {
            cursor: text.len(),
            text,
            ..Self::default()
        }
    }
}
impl From<&str> for Editor {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}
impl std::ops::Deref for Editor {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}
impl std::fmt::Display for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.text.fmt(f)
    }
}
impl PartialEq<&str> for Editor {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl Editor {
    pub fn as_str(&self) -> &str {
        &self.text
    }
    pub fn into_string(self) -> String {
        self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn selection(&self) -> Range<usize> {
        let anchor = self.anchor.unwrap_or(self.cursor);
        anchor.min(self.cursor)..anchor.max(self.cursor)
    }
    /// Reject the entire insertion on overflow; never send a silently cut paste.
    pub fn insert(&mut self, value: &str, max_bytes: usize) -> bool {
        let selected = self.selection();
        if self
            .text
            .len()
            .saturating_sub(selected.len())
            .saturating_add(value.len())
            > max_bytes
        {
            return false;
        }
        self.replace(selected.clone(), value);
        self.cursor = boundary_after(&self.text, selected.start + value.len());
        self.anchor = None;
        self.preferred_column = None;
        true
    }
    /// Replace an admitted source range atomically. Completion uses the same
    /// edit operation as typing, so folds, selection and limits stay coherent.
    pub fn replace_range(&mut self, range: Range<usize>, value: &str, max_bytes: usize) -> bool {
        if range.start > range.end
            || range.end > self.text.len()
            || boundary_after(&self.text, range.start) != range.start
            || boundary_after(&self.text, range.end) != range.end
            || self.text.len() - range.len() + value.len() > max_bytes
        {
            return false;
        }
        self.anchor = Some(range.start);
        self.cursor = range.end;
        self.insert(value, max_bytes)
    }
    pub fn folds(&self) -> &[Fold] {
        &self.folds
    }
    pub fn insert_folded(&mut self, value: &str, label: String, max_bytes: usize) -> bool {
        let start = self.selection().start;
        if !self.insert(value, max_bytes) {
            return false;
        }
        if !value.is_empty()
            && boundary_after(&self.text, start) == start
            && boundary_after(&self.text, start + value.len()) == start + value.len()
        {
            self.next_fold += 1;
            self.folds.push(Fold {
                id: self.next_fold,
                source: start..start + value.len(),
                label,
                expanded: false,
            });
            self.folds.sort_by_key(|fold| fold.source.start);
            *self.geometry.get_mut() = None;
        }
        true
    }
    /// Toggle the range at or immediately before the caret. Applications may
    /// also address an explicit ID through `set_expanded` for pointer actions.
    pub fn toggle_fold(&mut self) -> bool {
        let id = self
            .folds
            .iter()
            .rev()
            .find(|fold| fold.source.contains(&self.cursor) || fold.source.end == self.cursor)
            .map(|fold| fold.id);
        let Some(id) = id else {
            return false;
        };
        let expanded = !self
            .folds
            .iter()
            .find(|fold| fold.id == id)
            .unwrap()
            .expanded;
        self.set_expanded(id, expanded)
    }
    pub fn set_expanded(&mut self, id: u64, expanded: bool) -> bool {
        let Some(fold) = self.folds.iter_mut().find(|fold| fold.id == id) else {
            return false;
        };
        fold.expanded = expanded;
        *self.geometry.get_mut() = None;
        if !expanded && fold.source.contains(&self.cursor) {
            self.cursor = fold.source.start;
        }
        self.anchor = None;
        true
    }
    fn replace(&mut self, range: Range<usize>, value: &str) {
        *self.geometry.get_mut() = None;
        self.folds.retain_mut(|fold| {
            if fold.source.end <= range.start {
                return true;
            }
            if fold.source.start >= range.end {
                fold.source.start = fold.source.start - range.len() + value.len();
                fold.source.end = fold.source.end - range.len() + value.len();
                return true;
            }
            // Editing a range expands its source into ordinary editable text.
            false
        });
        self.text.replace_range(range, value);
        if !self.folds.is_empty() {
            let boundaries: std::collections::BTreeSet<_> = self
                .text
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .chain(std::iter::once(self.text.len()))
                .collect();
            self.folds.retain(|fold| {
                boundaries.contains(&fold.source.start) && boundaries.contains(&fold.source.end)
            });
        }
    }
    fn move_to(&mut self, cursor: usize, selecting: bool) {
        if selecting {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = boundary_after(&self.text, cursor.min(self.text.len()));
    }
    fn erase_to(&mut self, target: usize) {
        let selected = self.selection();
        let range = if selected.is_empty() {
            self.cursor.min(target)..self.cursor.max(target)
        } else {
            selected
        };
        self.replace(range.clone(), "");
        self.cursor = boundary_after(&self.text, range.start);
        self.anchor = None;
        self.preferred_column = None;
    }
    fn word_boundary(&self, forward: bool) -> usize {
        let target = if forward {
            self.text
                .unicode_word_indices()
                .map(|(start, word)| start + word.len())
                .find(|end| *end > self.cursor)
                .unwrap_or(self.text.len())
        } else {
            self.text
                .unicode_word_indices()
                .map(|(start, _)| start)
                .take_while(|start| *start < self.cursor)
                .last()
                .unwrap_or(0)
        };
        // A collapsed range is one visible unit, even when its source has words.
        self.folds
            .iter()
            .find(|fold| !fold.expanded && fold.source.start < target && target < fold.source.end)
            .map_or(target, |fold| {
                if forward {
                    fold.source.end
                } else {
                    fold.source.start
                }
            })
    }
    fn line_boundary(&self, width: u16, end: bool) -> usize {
        let layout = self.layout(width);
        let line = &layout.lines[layout.cursor.0];
        if end { line.end } else { line.start }
    }
    fn previous(&self) -> usize {
        if let Some(fold) = self
            .folds
            .iter()
            .find(|fold| !fold.expanded && fold.source.end == self.cursor)
        {
            return fold.source.start;
        }
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self) -> usize {
        if let Some(fold) = self
            .folds
            .iter()
            .find(|fold| !fold.expanded && fold.source.start == self.cursor)
        {
            return fold.source.end;
        }
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |g| self.cursor + g.len())
    }
    /// Editing keys only. Enter/Esc/Tab and application bindings are caller-owned.
    pub fn key(&mut self, key: KeyEvent, width: u16, max_bytes: usize) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        let selecting = key.modifiers.contains(KeyModifiers::SHIFT);
        let modifiers = key.modifiers.difference(KeyModifiers::SHIFT);
        let code = match key.code {
            KeyCode::Char(ch) => KeyCode::Char(ch.to_ascii_lowercase()),
            code => code,
        };
        let motion =
            match (code, modifiers) {
                (KeyCode::Home, KeyModifiers::CONTROL) => Some(0),
                (KeyCode::End, KeyModifiers::CONTROL) => Some(self.text.len()),
                (KeyCode::Char('a'), KeyModifiers::CONTROL)
                | (KeyCode::Left, KeyModifiers::SUPER) => Some(self.line_boundary(width, false)),
                (KeyCode::Char('e'), KeyModifiers::CONTROL)
                | (KeyCode::Right, KeyModifiers::SUPER) => Some(self.line_boundary(width, true)),
                (KeyCode::Left, KeyModifiers::ALT | KeyModifiers::CONTROL)
                | (KeyCode::Char('b'), KeyModifiers::ALT) => Some(self.word_boundary(false)),
                (KeyCode::Right, KeyModifiers::ALT | KeyModifiers::CONTROL)
                | (KeyCode::Char('f'), KeyModifiers::ALT) => Some(self.word_boundary(true)),
                _ => None,
            };
        if let Some(target) = motion {
            self.move_to(target, selecting);
            self.preferred_column = None;
            return true;
        }
        let deletion = match (code, modifiers) {
            (KeyCode::Backspace, KeyModifiers::ALT)
            | (KeyCode::Char('w'), KeyModifiers::CONTROL) => Some(self.word_boundary(false)),
            (KeyCode::Delete, KeyModifiers::ALT) | (KeyCode::Char('d'), KeyModifiers::ALT) => {
                Some(self.word_boundary(true))
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL)
            | (KeyCode::Backspace, KeyModifiers::SUPER) => Some(self.line_boundary(width, false)),
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => Some(self.line_boundary(width, true)),
            _ => None,
        };
        if let Some(target) = deletion {
            self.erase_to(target);
            return true;
        }
        if !modifiers.is_empty() {
            return false;
        }
        match key.code {
            KeyCode::Char(ch) if !ch.is_control() => {
                self.insert(&ch.to_string(), max_bytes);
            }
            KeyCode::Backspace => self.erase_to(self.previous()),
            KeyCode::Delete => self.erase_to(self.next()),
            KeyCode::Left | KeyCode::Right => {
                let range = self.selection();
                let at = if !selecting && !range.is_empty() {
                    if key.code == KeyCode::Left {
                        range.start
                    } else {
                        range.end
                    }
                } else if key.code == KeyCode::Left {
                    self.previous()
                } else {
                    self.next()
                };
                self.move_to(at, selecting);
                self.preferred_column = None;
            }
            KeyCode::Home | KeyCode::End => {
                self.move_to(
                    self.line_boundary(width, key.code == KeyCode::End),
                    selecting,
                );
                self.preferred_column = None;
            }
            KeyCode::Up | KeyCode::Down => {
                let layout = self.layout(width);
                let column = self.preferred_column.unwrap_or(layout.cursor.1);
                let row = if key.code == KeyCode::Up {
                    layout.cursor.0.saturating_sub(1)
                } else {
                    (layout.cursor.0 + 1).min(layout.lines.len() - 1)
                };
                self.move_to(layout.offset(row, column), selecting);
                self.preferred_column = Some(column);
            }
            _ => return false,
        }
        true
    }
    pub fn place(&mut self, width: u16, row: usize, column: usize, selecting: bool) {
        let offset = self.layout(width).offset(row, column);
        self.move_to(offset, selecting);
        self.preferred_column = None;
    }
    /// Hard wraps without dropping spaces. Raw source offsets survive escaping,
    /// wide/combining graphemes, explicit newlines and resizing.
    pub fn layout(&self, width: u16) -> EditorLayout {
        let width = width.max(1);
        let lines = {
            let mut geometry = self.geometry.borrow_mut();
            if geometry
                .as_ref()
                .is_none_or(|(old_width, _)| *old_width != width)
            {
                *geometry = Some((width, Rc::new(self.build_lines(width))));
            }
            geometry.as_ref().unwrap().1.clone()
        };
        let row = lines
            .partition_point(|line| line.start <= self.cursor)
            .saturating_sub(1);
        let column = lines[row]
            .cells
            .iter()
            .take_while(|cell| cell.source.end <= self.cursor)
            .map(|cell| cell.width)
            .sum();
        EditorLayout {
            lines,
            cursor: (row, column),
        }
    }
    fn build_lines(&self, width: u16) -> Vec<EditorLine> {
        let width = usize::from(width);
        let mut lines = vec![EditorLine::default()];
        let mut column = 0;
        let mut start = 0;
        while start < self.text.len() {
            let fold = self
                .folds
                .iter()
                .find(|fold| !fold.expanded && fold.source.start == start);
            let grapheme = if let Some(fold) = fold {
                &self.text[start..fold.source.end]
            } else {
                self.text[start..].graphemes(true).next().unwrap()
            };
            let end = start + grapheme.len();
            if fold.is_none() && matches!(grapheme, "\n" | "\r\n" | "\r") {
                lines.last_mut().expect("line").end = start;
                lines.push(EditorLine {
                    start: end,
                    end,
                    cells: Vec::new(),
                });
                column = 0;
                start = end;
                continue;
            }
            let mut text = safe_grapheme(fold.map_or(grapheme, |fold| fold.label.as_str()));
            if fold.is_some() && text.width() > width {
                let mut used = 0;
                text = text
                    .graphemes(true)
                    .take_while(|g| {
                        used += g.width();
                        used < width
                    })
                    .collect::<String>()
                    + "…";
            }
            let mut cells = text.width();
            if cells == 0 {
                text.insert(0, '◌');
                cells = text.width();
            }
            if cells > width {
                text = "�".into();
                cells = 1;
            }
            if column + cells > width || column == width {
                lines.push(EditorLine {
                    start,
                    end: start,
                    cells: Vec::new(),
                });
                column = 0;
            }
            let line = lines.last_mut().expect("line");
            line.cells.push(EditorCell {
                text,
                source: start..end,
                column,
                width: cells,
            });
            line.end = end;
            column += cells;
            start = end;
        }
        if column == width {
            lines.push(EditorLine {
                start: self.text.len(),
                end: self.text.len(),
                cells: Vec::new(),
            });
        }
        lines
    }
}

impl EditorLayout {
    /// Paint visible source cells through the same strip owner as other TUI
    /// text. Applications retain viewport/focus policy and terminal presentation.
    pub fn paint(
        &self,
        buffer: &mut ratatui::buffer::Buffer,
        area: ratatui::layout::Rect,
        offset: usize,
        selected: Range<usize>,
        base: ratatui::style::Style,
        highlight: ratatui::style::Style,
    ) -> Option<ratatui::layout::Position> {
        use ratatui::{
            layout::Rect,
            text::{Line, Span},
        };
        for (row, line) in self
            .lines
            .iter()
            .skip(offset)
            .take(usize::from(area.height))
            .enumerate()
        {
            let spans = line
                .cells
                .iter()
                .map(|cell| {
                    Span::styled(
                        cell.text.clone(),
                        if cell.source.start < selected.end && cell.source.end > selected.start {
                            highlight
                        } else {
                            base
                        },
                    )
                })
                .collect::<Vec<_>>();
            super::strip::paint_left(
                buffer,
                Rect {
                    y: area.y + row as u16,
                    height: 1,
                    ..area
                },
                Line::from(spans),
            );
        }
        (self.cursor.0 >= offset
            && self.cursor.0 - offset < usize::from(area.height)
            && self.cursor.1 < usize::from(area.width))
        .then(|| {
            (
                area.x + self.cursor.1 as u16,
                area.y + (self.cursor.0 - offset) as u16,
            )
                .into()
        })
    }
    pub fn offset(&self, row: usize, column: usize) -> usize {
        let line = &self.lines[row.min(self.lines.len() - 1)];
        for cell in &line.cells {
            if column < cell.column + cell.width {
                return if column.saturating_sub(cell.column) * 2 < cell.width {
                    cell.source.start
                } else {
                    cell.source.end
                };
            }
        }
        line.end
    }
    /// Smallest scrolling change that keeps the caret inside the viewport.
    pub fn reveal(&self, offset: usize, height: u16) -> usize {
        let height = usize::from(height.max(1));
        offset
            .min(self.cursor.0)
            .max(self.cursor.0.saturating_add(1).saturating_sub(height))
            .min(self.lines.len().saturating_sub(height))
    }
}
fn boundary_after(text: &str, at: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .find(|i| *i >= at)
        .unwrap_or(text.len())
}
fn safe_grapheme(value: &str) -> String {
    let visible: String = value.chars().flat_map(|ch| {
        if matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            format!("\\u{{{:x}}}", u32::from(ch)).chars().collect::<Vec<_>>()
        } else { vec![ch] }
    }).collect();
    escape(&visible)
}

#[cfg(test)]
mod tests;
