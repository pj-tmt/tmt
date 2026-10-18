//! Fitted tables use the same inline parser and member-link projection as prose.
use super::*;

impl Renderer<'_> {
    pub(super) fn table(&mut self, range: std::ops::Range<usize>) {
        let source = self.source;
        let raw = &source[range.clone()];
        let old_heading = self.heading;
        let mut head = false;
        let mut row = Vec::new();
        let mut rows = Vec::new();
        let mut row_source = self.source_line;
        for (event, at) in Parser::new_ext(raw, Options::ENABLE_TABLES).into_offset_iter() {
            let at = range.start + at.start..range.start + at.end;
            match event {
                Event::Start(Tag::TableHead) => {
                    head = true;
                    row_source = self
                        .starts
                        .partition_point(|start| *start <= at.start)
                        .saturating_sub(1);
                }
                Event::Start(Tag::TableRow) => {
                    row_source = self
                        .starts
                        .partition_point(|start| *start <= at.start)
                        .saturating_sub(1);
                }
                Event::Start(Tag::TableCell) => {
                    self.heading = old_heading || head;
                    self.source_line = row_source;
                    self.start(String::new(), 0);
                }
                Event::End(TagEnd::TableCell) => {
                    row.push(
                        self.current
                            .take()
                            .map_or_else(Vec::new, |block| block.spans),
                    );
                }
                Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                    rows.push((row_source, head, std::mem::take(&mut row)));
                    head = false;
                }
                Event::Text(_)
                | Event::Code(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::Start(
                    Tag::Strong | Tag::Emphasis | Tag::Link { .. } | Tag::Image { .. },
                )
                | Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link)
                | Event::SoftBreak
                | Event::HardBreak => self.event(event, at),
                _ => {}
            }
        }
        self.heading = old_heading;
        self.current = None;
        let columns = rows
            .iter()
            .map(|(_, _, cells)| cells.len())
            .max()
            .unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|column| {
                rows.iter()
                    .filter_map(|(_, _, row)| row.get(column))
                    .map(|parts| parts.iter().map(|part| cells(&part.text)).sum())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let required = widths
            .iter()
            .sum::<usize>()
            .saturating_add(columns.saturating_mul(3))
            .saturating_add(1);
        if columns == 0 || required > self.width {
            self.source_line = self
                .starts
                .partition_point(|start| *start <= range.start)
                .saturating_sub(1);
            self.source(range);
            return;
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
        self.table_line(
            vec![Part::new(border('┌', '┬', '┐'), rule, first_source, None)],
            first_source,
        );
        for (source, header, row) in rows {
            let mut parts = vec![Part::new("│".into(), rule, source, None)];
            let mut columns = row.into_iter();
            for column_width in &widths {
                let mut content = columns.next().unwrap_or_default();
                let width = content.iter().map(|part| cells(&part.text)).sum::<usize>();
                parts.push(Part::new(" ".into(), text_style, source, None));
                parts.append(&mut content);
                parts.push(Part::new(
                    " ".repeat(column_width - width + 1),
                    text_style,
                    source,
                    None,
                ));
                parts.push(Part::new("│".into(), rule, source, None));
            }
            self.table_line(parts, source);
            if header {
                self.table_line(
                    vec![Part::new(border('├', '┼', '┤'), rule, source, None)],
                    source,
                );
            }
        }
        self.table_line(
            vec![Part::new(border('└', '┴', '┘'), rule, last_source, None)],
            last_source,
        );
        self.skip_until = range.end;
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
