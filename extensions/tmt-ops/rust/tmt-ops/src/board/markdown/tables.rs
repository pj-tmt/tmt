//! Fitted tables use the same inline parser and member-link projection as prose.
//! A table wider than the pane shrinks its widest columns and wraps their cells; its source
//! is shown as written only when even a two-cell column cannot fit.
use super::*;

/// The least a column keeps when a wide table is squeezed into the pane.
const COLUMN_FLOOR: usize = 2;

/// A table being read: its source range and the rows closed so far.
pub(super) struct Build {
    range: std::ops::Range<usize>,
    heading: bool,
    head: bool,
    row_source: usize,
    row: Vec<Vec<Part>>,
    rows: Vec<(usize, bool, Vec<Vec<Part>>)>,
}

impl Renderer<'_> {
    pub(super) fn table_start(&mut self, range: std::ops::Range<usize>) {
        self.table = Some(Build {
            range,
            heading: self.heading,
            head: false,
            row_source: self.source_line,
            row: Vec::new(),
            rows: Vec::new(),
        });
    }

    fn line_of(&self, at: usize) -> usize {
        self.starts
            .partition_point(|start| *start <= at)
            .saturating_sub(1)
    }

    /// Reads the table structure from the main event stream; inline events inside a cell go
    /// through the ordinary handlers. Returns whether the event was consumed.
    pub(super) fn table_event(&mut self, event: &Event<'_>, at: &std::ops::Range<usize>) -> bool {
        let line = self.line_of(at.start);
        let Some(table) = self.table.as_mut() else {
            return false;
        };
        match event {
            Event::Start(Tag::TableHead) => {
                table.head = true;
                table.row_source = line;
            }
            Event::Start(Tag::TableRow) => table.row_source = line,
            Event::Start(Tag::TableCell) => {
                let (heading, row_source) = (table.heading || table.head, table.row_source);
                self.heading = heading;
                self.source_line = row_source;
                self.start(String::new(), 0);
            }
            Event::End(TagEnd::TableCell) => {
                let cell = self
                    .current
                    .take()
                    .map_or_else(Vec::new, |block| block.spans);
                table.row.push(cell);
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                let row = std::mem::take(&mut table.row);
                table.rows.push((table.row_source, table.head, row));
                table.head = false;
            }
            Event::End(TagEnd::Table) => {
                let table = self.table.take().expect("a table is open");
                self.heading = table.heading;
                self.current = None;
                self.lay_out(table);
            }
            // A cell is one line of prose; its breaks read as spaces.
            Event::SoftBreak | Event::HardBreak => self.push(" ", self.style()),
            _ => return false,
        }
        true
    }

    fn lay_out(&mut self, table: Build) {
        let Build { range, rows, .. } = table;
        let columns = rows
            .iter()
            .map(|(_, _, cells)| cells.len())
            .max()
            .unwrap_or(0);
        let natural: Vec<usize> = (0..columns)
            .map(|column| {
                rows.iter()
                    .filter_map(|(_, _, row)| row.get(column))
                    .map(|parts| parts.iter().map(|part| cells(&part.text)).sum())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let quote = self.quote_depth * 2;
        let indent = self.lists.len() * 2;
        let overhead = columns.saturating_mul(3).saturating_add(1);
        let available = self.width.saturating_sub(quote + indent);
        let floors: Vec<usize> = natural
            .iter()
            .map(|width| (*width).min(COLUMN_FLOOR))
            .collect();
        if columns == 0 || floors.iter().sum::<usize>().saturating_add(overhead) > available {
            self.source_line = self.line_of(range.start);
            self.source(range);
            return;
        }
        let mut widths = natural;
        while widths.iter().sum::<usize>() + overhead > available {
            let widest = (0..columns)
                .filter(|column| widths[*column] > floors[*column])
                .max_by_key(|column| (widths[*column], std::cmp::Reverse(*column)))
                .expect("the floors fit, so a column above its floor remains");
            widths[widest] -= 1;
        }
        let rule = self.look.role(Role::Dim);
        let text_style = self.look.role(Role::Text);
        let border = |left: char, join: char, right: char| {
            let mut text = left.to_string();
            for (index, width) in widths.iter().enumerate() {
                if index > 0 {
                    text.push(join);
                }
                text.push_str(&"─".repeat(width + 2));
            }
            text.push(right);
            text
        };
        let first_source = rows[0].0;
        let last_source = rows.last().unwrap().0;
        let lead = |source: usize, this: &Self| {
            let mut parts = Vec::new();
            if this.quote_depth > 0 {
                parts.push(Part::new(
                    "▎ ".repeat(this.quote_depth),
                    this.look.role(Role::Dim),
                    source,
                    None,
                ));
            }
            if indent > 0 {
                parts.push(Part::new(" ".repeat(indent), text_style, source, None));
            }
            parts
        };
        let mut top = lead(first_source, self);
        top.push(Part::new(border('┌', '┬', '┐'), rule, first_source, None));
        self.table_line(top, first_source);
        for (source, header, row) in rows {
            let mut cells_wrapped: Vec<Vec<Vec<Part>>> = row
                .into_iter()
                .zip(&widths)
                .map(|(cell, width)| wrap_parts(cell, *width))
                .collect();
            cells_wrapped.resize_with(columns, || vec![Vec::new()]);
            let height = cells_wrapped.iter().map(Vec::len).max().unwrap_or(1);
            for visual in 0..height {
                let mut parts = lead(source, self);
                parts.push(Part::new("│".into(), rule, source, None));
                for (column, column_width) in widths.iter().enumerate() {
                    let mut content = cells_wrapped[column]
                        .get_mut(visual)
                        .map(std::mem::take)
                        .unwrap_or_default();
                    let used = content.iter().map(|part| cells(&part.text)).sum::<usize>();
                    parts.push(Part::new(" ".into(), text_style, source, None));
                    parts.append(&mut content);
                    parts.push(Part::new(
                        " ".repeat(column_width.saturating_sub(used) + 1),
                        text_style,
                        source,
                        None,
                    ));
                    parts.push(Part::new("│".into(), rule, source, None));
                }
                self.table_line(parts, source);
            }
            if header {
                let mut parts = lead(source, self);
                parts.push(Part::new(border('├', '┼', '┤'), rule, source, None));
                self.table_line(parts, source);
            }
        }
        let mut bottom = lead(last_source, self);
        bottom.push(Part::new(border('└', '┴', '┘'), rule, last_source, None));
        self.table_line(bottom, last_source);
    }

    fn table_line(&mut self, spans: Vec<Part>, source_line: usize) {
        self.blocks.push(Block {
            prefix: String::new(),
            indent: 0,
            spans,
            source_line,
            literal: true,
            style: Style::new(),
            prefix_style: Style::new(),
            quote_depth: 0,
        });
    }
}

/// Greedy word wrap of one cell into lines of at most `width` cells; a word longer than the
/// column is split between characters. Every piece keeps its style, source line and member link.
fn wrap_parts(parts: Vec<Part>, width: usize) -> Vec<Vec<Part>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<(usize, String)>> = vec![Vec::new()];
    let mut used = 0;
    let add = |lines: &mut Vec<Vec<(usize, String)>>, part: usize, text: &str| {
        let line = lines.last_mut().expect("a line is open");
        match line.last_mut() {
            Some((last, piece)) if *last == part => piece.push_str(text),
            _ => line.push((part, text.to_owned())),
        }
    };
    for (index, part) in parts.iter().enumerate() {
        for word in part.text.split_inclusive(' ') {
            let body = word.trim_end_matches(' ');
            if used > 0 && used + cells(body) > width {
                lines.push(Vec::new());
                used = 0;
            }
            for character in body.chars() {
                let character = if character.width().unwrap_or(0) > width {
                    '…'
                } else {
                    character
                };
                let size = character.width().unwrap_or(0);
                if used > 0 && used + size > width {
                    lines.push(Vec::new());
                    used = 0;
                }
                add(&mut lines, index, character.encode_utf8(&mut [0; 4]));
                used += size;
            }
            if word.ends_with(' ') && used > 0 && used < width {
                add(&mut lines, index, " ");
                used += 1;
            }
        }
    }
    lines
        .into_iter()
        .map(|line| {
            line.into_iter()
                .map(|(index, text)| {
                    let part = &parts[index];
                    Part::new(text, part.style, part.source, part.member.clone())
                })
                .collect()
        })
        .collect()
}
