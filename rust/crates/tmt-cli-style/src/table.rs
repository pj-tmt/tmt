//! Borderless, header-free rows on `comfy-table`. Widths are measured on
//! escaped plain text; color is applied after layout. With a known terminal
//! width, detail columns (paths, previews) give way before names, and every
//! row stays on one line, truncated with `…`. Without one nothing is cut.
//!
//! comfy-table fits every cell as plain text, one column at a time, and
//! [`Terminal::paint`] colors the fitted text afterward, so a token is the
//! same escape sequence in a table and beside it, and its style covers the
//! text, never the column padding. The one piece of custom layout is
//! `layout`, which chooses each column's width, because comfy-table cannot
//! express two of the style's rules: widths shared across the separate tables
//! of a list's sections, and a priority order in which detail columns give way
//! completely before names shrink.
//! `ContentArrangement::Dynamic` distributes the shortfall across columns
//! instead of prioritizing, and it lays out one table at a time. The computed
//! widths are handed to comfy-table as absolute column constraints.

use crate::{
    grid::{self, Track},
    palette::{Terminal, Token},
};
use comfy_table::{ColumnConstraint, Row, Width, presets};
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

/// Space between columns.
const GAP: usize = 2;
/// The narrowest a truncated column becomes, including its `…`.
const MINIMUM: usize = 4;

/// User data is untrusted: control and line-separator characters are shown
/// escaped, never interpreted by the terminal. This is a trust boundary.
pub fn escape(value: &str) -> String {
    let mut rendered = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}') {
            rendered.extend(ch.escape_default());
        } else {
            rendered.push(ch);
        }
    }
    rendered
}

/// How a column behaves when the terminal is too narrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    /// Never truncated: marks, states, short codes.
    Fixed,
    /// Truncated only after every detail column is at its minimum.
    Name,
    /// Truncated first: paths, previews, descriptions.
    Detail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    text: String,
    token: Option<Token>,
}

impl Cell {
    pub fn styled(text: impl AsRef<str>, token: Token) -> Self {
        Self {
            text: escape(text.as_ref()),
            token: Some(token),
        }
    }
}

impl<T: AsRef<str>> From<T> for Cell {
    fn from(text: T) -> Self {
        Self {
            text: escape(text.as_ref()),
            token: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    columns: Vec<Column>,
    rows: Vec<Vec<Cell>>,
    /// A trailing action per row, such as `↻ tmt resume <name>`.
    actions: Vec<Option<Cell>>,
    indent: usize,
}

impl Table {
    /// Rows indented two spaces, as in a list section.
    ///
    /// # Panics
    /// Without columns: a caller bug.
    pub fn new(columns: &[Column]) -> Self {
        assert!(!columns.is_empty(), "a table needs a column");
        Self {
            columns: columns.to_vec(),
            rows: Vec::new(),
            actions: Vec::new(),
            indent: 2,
        }
    }

    pub fn indent(mut self, indent: usize) -> Self {
        self.indent = indent;
        self
    }

    /// # Panics
    /// When the row does not have one cell per column: a caller bug.
    pub fn row<C: Into<Cell>>(&mut self, cells: impl IntoIterator<Item = C>) -> &mut Self {
        let cells: Vec<Cell> = cells.into_iter().map(Into::into).collect();
        assert_eq!(cells.len(), self.columns.len(), "one cell per column");
        self.rows.push(cells);
        self.actions.push(None);
        self
    }

    /// A row with a trailing action, shown only where an action is possible:
    /// accent-colored after every column, and never truncated.
    ///
    /// # Panics
    /// When the row does not have one cell per column: a caller bug.
    pub fn row_with_action<C: Into<Cell>>(
        &mut self,
        cells: impl IntoIterator<Item = C>,
        action: &str,
    ) -> &mut Self {
        self.row(cells);
        *self.actions.last_mut().expect("the row was just added") =
            Some(Cell::styled(action, Token::Accent));
        self
    }

    /// A row with a trailing note, a state worth reading beside the row where no
    /// action is possible: dimmed, in the action's slot after every column, and
    /// never truncated. A row carries an action or a note, not both.
    ///
    /// # Panics
    /// When the row does not have one cell per column: a caller bug.
    pub fn row_with_note<C: Into<Cell>>(
        &mut self,
        cells: impl IntoIterator<Item = C>,
        note: &str,
    ) -> &mut Self {
        self.row(cells);
        *self.actions.last_mut().expect("the row was just added") =
            Some(Cell::styled(note, Token::Dim));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn write(&self, output: &mut impl Write, terminal: Terminal) -> io::Result<()> {
        self.write_laid_out(output, terminal, &layout(&[self], terminal.width))
    }

    pub(crate) fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub(crate) fn write_laid_out(
        &self,
        output: &mut impl Write,
        terminal: Terminal,
        widths: &[usize],
    ) -> io::Result<()> {
        if self.rows.is_empty() {
            return Ok(());
        }
        let with_actions = widths.len() > self.columns.len();
        let none = Cell::from("");
        let rows: Vec<Vec<&Cell>> = self
            .rows
            .iter()
            .zip(&self.actions)
            .map(|(cells, action)| {
                let action = with_actions.then(|| action.as_ref().unwrap_or(&none));
                cells.iter().chain(action).collect()
            })
            .collect();
        let last = widths.len() - 1;
        let columns: Vec<Vec<String>> = widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                let gap = if index == last { 0 } else { GAP };
                fitted(rows.iter().map(|row| row[index]), *width, gap)
            })
            .collect();
        let indent = " ".repeat(self.indent);
        for (index, row) in rows.iter().enumerate() {
            let mut line = String::new();
            for (cell, column) in row.iter().zip(&columns) {
                let fitted = &column[index];
                let text = fitted.trim_end_matches(' ');
                match cell.token {
                    Some(token) => line.push_str(&terminal.paint(token, text)),
                    None => line.push_str(text),
                }
                line.push_str(&" ".repeat(fitted.len() - text.len()));
            }
            let line = line.trim_end();
            if line.is_empty() {
                writeln!(output)?;
            } else {
                writeln!(output, "{indent}{line}")?;
            }
        }
        Ok(())
    }
}

/// One column's cells as plain text, each exactly as comfy-table fits it at
/// `width`: cut with `…` (whole graphemes, at a word boundary where it can)
/// and padded by `gap`. Color is painted afterward, so it never counts toward
/// the width and never covers the padding.
fn fitted<'a>(cells: impl Iterator<Item = &'a Cell>, width: usize, gap: usize) -> Vec<String> {
    let mut table = comfy_table::Table::new();
    table
        .load_style(presets::NOTHING)
        .set_truncation_indicator("…")
        .force_no_tty();
    for cell in cells {
        let mut row = Row::new();
        row.max_height(1);
        row.add_cell(comfy_table::Cell::new(&cell.text));
        table.add_row(row);
    }
    if let Some(column) = table.column_mut(0) {
        column.set_padding((0, gap as u16));
        column.set_constraint(ColumnConstraint::Absolute(Width::Fixed(
            (width + gap) as u16,
        )));
    }
    table.lines().collect()
}

/// Column widths shared by tables with the same columns and indent: natural
/// widths, then detail columns and afterwards names shrink, widest first,
/// until the rows fit or nothing more may shrink. When any row has an action,
/// one more fixed column holds the actions. The widths come from the one
/// [`grid`](crate::grid) solver: detail and name columns are its first and
/// second shrink tiers, and fixed columns never shrink.
pub(crate) fn layout(tables: &[&Table], available: Option<u16>) -> Vec<usize> {
    let first = tables[0];
    let natural = |index: usize| {
        tables
            .iter()
            .flat_map(|table| &table.rows)
            .map(|row| row[index].text.width())
            .max()
            .unwrap_or(0)
    };
    let mut tracks: Vec<Track> = first
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let basis = natural(index);
            match column {
                Column::Fixed => Track::fixed(basis),
                Column::Detail | Column::Name => Track {
                    basis: grid::Basis::Cells(basis),
                    min: basis.min(MINIMUM),
                    max: None,
                    grow: 0,
                    shrink: u8::from(*column == Column::Name),
                    priority: None,
                },
            }
        })
        .collect();
    let actions = tables
        .iter()
        .flat_map(|table| table.actions.iter().flatten())
        .map(|action| action.text.width())
        .max();
    if let Some(width) = actions {
        tracks.push(Track::fixed(width));
    }
    let available = available.map(|width| usize::from(width).saturating_sub(first.indent));
    grid::solve(&tracks, available, GAP)
        .into_iter()
        .map(|width| width.expect("table columns never step aside"))
        .collect()
}

#[cfg(test)]
mod tests;
