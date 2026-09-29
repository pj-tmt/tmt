//! Borderless, header-free rows on `comfy-table`. Widths are measured on
//! escaped plain text; color is applied after layout. With a known terminal
//! width, detail columns (paths, previews) give way before names, and every
//! row stays on one line, truncated with `…`. Without one nothing is cut.
//!
//! comfy-table renders, truncates and styles every cell. The one piece of
//! custom layout is `layout`, which chooses each column's width, because
//! comfy-table cannot express two of the style's rules: widths shared across
//! the separate tables of a list's sections, and a priority order in which
//! detail columns give way completely before names shrink.
//! `ContentArrangement::Dynamic` distributes the shortfall across columns
//! instead of prioritizing, and it lays out one table at a time. The computed
//! widths are handed to comfy-table as absolute column constraints.

use crate::palette::{Terminal, Token};
use comfy_table::{Attribute, Color, ColumnConstraint, Row, Width, presets};
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
        let mut table = comfy_table::Table::new();
        table
            .load_style(presets::NOTHING)
            .set_truncation_indicator("…")
            .force_no_tty();
        if terminal.color {
            table.enforce_styling();
        }
        let with_actions = widths.len() > self.columns.len();
        for (cells, action) in self.rows.iter().zip(&self.actions) {
            let mut row = Row::new();
            row.max_height(1);
            for cell in cells {
                row.add_cell(styled(cell));
            }
            if with_actions {
                let none = Cell::from("");
                row.add_cell(styled(action.as_ref().unwrap_or(&none)));
            }
            table.add_row(row);
        }
        let last = widths.len() - 1;
        for (index, column) in table.column_iter_mut().enumerate() {
            let gap = if index == last { 0 } else { GAP };
            column.set_padding((0, gap as u16));
            column.set_constraint(ColumnConstraint::Absolute(Width::Fixed(
                (widths[index] + gap) as u16,
            )));
        }
        let indent = " ".repeat(self.indent);
        for line in table.lines() {
            let line = without_trailing_padding(&line);
            if line.is_empty() {
                writeln!(output)?;
            } else {
                writeln!(output, "{indent}{line}")?;
            }
        }
        Ok(())
    }
}

/// Column widths shared by tables with the same columns and indent: natural
/// widths, then detail columns and afterwards names shrink, widest first,
/// until the rows fit or nothing more may shrink. When any row has an action,
/// one more fixed column holds the actions.
pub(crate) fn layout(tables: &[&Table], available: Option<u16>) -> Vec<usize> {
    let first = tables[0];
    let mut columns = first.columns.clone();
    let mut widths: Vec<usize> = (0..columns.len())
        .map(|index| {
            tables
                .iter()
                .flat_map(|table| &table.rows)
                .map(|row| row[index].text.width())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let actions = tables
        .iter()
        .flat_map(|table| table.actions.iter().flatten())
        .map(|action| action.text.width())
        .max();
    if let Some(width) = actions {
        columns.push(Column::Fixed);
        widths.push(width);
    }
    let Some(available) = available else {
        return widths;
    };
    let budget = usize::from(available).saturating_sub(first.indent);
    let total = |widths: &[usize]| widths.iter().sum::<usize>() + GAP * (widths.len() - 1);
    for kind in [Column::Detail, Column::Name] {
        while total(&widths) > budget {
            let widest = (0..widths.len())
                .filter(|&index| columns[index] == kind && widths[index] > MINIMUM)
                .max_by_key(|&index| (widths[index], std::cmp::Reverse(index)));
            let Some(index) = widest else { break };
            let excess = total(&widths) - budget;
            let next = (0..widths.len())
                .filter(|&other| other != index && columns[other] == kind)
                .map(|other| widths[other])
                .filter(|&width| width < widths[index])
                .max()
                .unwrap_or(0)
                .max(MINIMUM);
            widths[index] -= excess.min(widths[index] - next).max(1);
        }
    }
    widths
}

/// comfy-table pads a styled cell inside its color span, so padding can sit
/// before the closing reset codes; drop it there as well as at the end.
fn without_trailing_padding(line: &str) -> String {
    let mut end = line.trim_end();
    let mut resets = Vec::new();
    while let Some(start) = end.rfind('\u{1b}') {
        let code = &end[start..];
        if !(code.starts_with("\u{1b}[") && code.ends_with('m')) {
            break;
        }
        resets.push(code);
        end = end[..start].trim_end();
    }
    resets.reverse();
    format!("{end}{}", resets.concat())
}

fn styled(cell: &Cell) -> comfy_table::Cell {
    let mut rendered = comfy_table::Cell::new(&cell.text);
    let Some(token) = cell.token else {
        return rendered;
    };
    if let Some(color) = token.color().map(dark) {
        rendered = rendered.fg(color);
    }
    match token {
        Token::Dim | Token::OtherDriver => rendered.add_attribute(Attribute::Dim),
        Token::Title | Token::Literal => rendered.add_attribute(Attribute::Bold),
        _ => rendered,
    }
}

/// crossterm's "dark" names are the standard (non-bright) palette entries
/// 1–6, the same entries the token names in [`Token::color`].
fn dark(color: anstyle::AnsiColor) -> Color {
    use anstyle::AnsiColor as Ansi;
    match color {
        Ansi::Red => Color::DarkRed,
        Ansi::Green => Color::DarkGreen,
        Ansi::Yellow => Color::DarkYellow,
        Ansi::Blue => Color::DarkBlue,
        Ansi::Magenta => Color::DarkMagenta,
        Ansi::Cyan => Color::DarkCyan,
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod tests;
