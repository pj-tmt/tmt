//! A list section: an UPPERCASE bold title with a dimmed count, rows indented
//! two spaces with no header row or borders, an optional dimmed note for rows
//! folded away, then a hint only when there is something to do next.

use crate::{
    message,
    palette::{Terminal, Token},
    table::{self, Table},
};
use std::io::{self, Write};

pub struct Section<'a> {
    pub title: &'a str,
    /// Shown dimmed after the title; `None` for sections that are not counts.
    pub count: Option<usize>,
    pub rows: Table,
    /// One dimmed line under the rows, such as `offline: a · b` for rows that
    /// were folded away.
    pub note: Option<&'a str>,
    pub hint: Option<&'a str>,
}

impl Section<'_> {
    pub fn write(&self, output: &mut impl Write, terminal: Terminal) -> io::Result<()> {
        let widths = table::layout(&[&self.rows], terminal.width);
        self.write_laid_out(output, terminal, &widths)
    }

    fn write_laid_out(
        &self,
        output: &mut impl Write,
        terminal: Terminal,
        widths: &[usize],
    ) -> io::Result<()> {
        let title = terminal.paint(Token::Title, &self.title.to_uppercase());
        match self.count {
            Some(count) => writeln!(
                output,
                "{title} {}",
                terminal.paint(Token::Dim, &count.to_string())
            )?,
            None => writeln!(output, "{title}")?,
        }
        self.rows.write_laid_out(output, terminal, widths)?;
        if let Some(note) = self.note {
            writeln!(output, "    {}", terminal.paint(Token::Dim, note))?;
        }
        match self.hint {
            Some(next) => message::hint(output, terminal, next),
            None => Ok(()),
        }
    }
}

/// Sections separated by one blank line. Sections with the same columns
/// share one layout, so their rows line up.
pub fn write(
    output: &mut impl Write,
    terminal: Terminal,
    sections: &[Section<'_>],
) -> io::Result<()> {
    let tables: Vec<&Table> = sections
        .iter()
        .map(|section| &section.rows)
        .filter(|rows| !rows.is_empty())
        .collect();
    let shared = tables
        .first()
        .filter(|first| {
            tables
                .iter()
                .all(|table| table.columns() == first.columns())
        })
        .map(|_| table::layout(&tables, terminal.width));
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            writeln!(output)?;
        }
        match &shared {
            Some(widths) if !section.rows.is_empty() => {
                section.write_laid_out(output, terminal, widths)?
            }
            _ => section.write(output, terminal)?,
        }
    }
    Ok(())
}
