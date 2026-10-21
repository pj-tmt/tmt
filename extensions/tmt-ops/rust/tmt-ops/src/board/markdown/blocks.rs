//! Structured block projections retain original source-line identities.
use super::*;

impl Renderer<'_> {
    pub(super) fn code_block(&mut self, range: std::ops::Range<usize>) {
        let raw = &self.source[range.clone()];
        let style = self
            .look
            .role(Role::Code)
            .patch(self.look.role(Role::Track));
        let mut painted = false;
        for (event, at) in Parser::new(raw).into_offset_iter() {
            let Event::Text(body) = event else { continue };
            let first = self
                .starts
                .partition_point(|start| *start <= range.start + at.start)
                .saturating_sub(1);
            for (offset, line) in body
                .strip_suffix('\n')
                .unwrap_or(&body)
                .split('\n')
                .enumerate()
            {
                let text = crate::board::view::fit(line, self.width);
                self.blocks.push(Block {
                    prefix: String::new(),
                    indent: 0,
                    spans: vec![Part::new(text, style, first + offset, None)],
                    source_line: first + offset,
                    literal: true,
                    style,
                    prefix_style: Style::new(),
                    quote_depth: 0,
                });
                painted = true;
            }
        }
        if !painted {
            self.blocks.push(Block {
                prefix: String::new(),
                indent: 0,
                spans: vec![Part::new(
                    " ".repeat(self.width),
                    style,
                    self.source_line,
                    None,
                )],
                source_line: self.source_line,
                literal: true,
                style,
                prefix_style: Style::new(),
                quote_depth: 0,
            });
        }
        self.skip_until = range.end;
    }
}
