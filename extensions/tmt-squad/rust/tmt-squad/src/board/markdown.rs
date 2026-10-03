//! A thin Markdown view of already-sanitized notes. Only headings, lists,
//! bold, italic, inline code and links get styles; every other construct
//! (tables, HTML, images, code blocks, quotes, footnotes) is shown as its own
//! source text, never dropped or half-rendered. Nothing here executes,
//! fetches or opens anything; links carry inert destination metadata.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthChar;

/// One logical line before wrapping: a first-line prefix, the indent for
/// wrapped continuation lines, and styled text.
struct Block {
    prefix: String,
    indent: usize,
    spans: Vec<(String, Style, usize, Option<usize>)>,
    source_line: usize,
}

struct Renderer<'a> {
    source: &'a str,
    starts: Vec<usize>,
    source_line: usize,
    look: crate::look::Look,
    blocks: Vec<Block>,
    current: Option<Block>,
    bold: usize,
    italic: usize,
    heading: bool,
    link: Option<usize>,
    links: Vec<Link>,
    destination: Option<String>,
    handlers: &'a crate::links::Handlers,
    /// Numbering for each open list; None for bullets.
    lists: Vec<Option<u64>>,
    /// Skip events inside an unsupported construct already shown as source.
    skip_until: usize,
}

impl Renderer<'_> {
    fn style(&self) -> Style {
        let mut style = Style::new();
        if self.bold > 0 || self.heading {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.link.is_some() {
            style = style
                .patch(self.look.role(Role::Link))
                .add_modifier(Modifier::UNDERLINED);
        }
        style
    }

    fn start(&mut self, prefix: String, indent: usize) {
        self.finish();
        self.current = Some(Block {
            prefix,
            indent,
            spans: Vec::new(),
            source_line: self.source_line,
        });
    }

    fn finish(&mut self) {
        if let Some(block) = self.current.take() {
            self.blocks.push(block);
        }
    }

    fn push(&mut self, text: &str, style: Style) {
        if self.current.is_none() {
            let indent = self.lists.len() * 2;
            self.start(" ".repeat(indent), indent);
        }
        if let Some(block) = &mut self.current {
            block
                .spans
                .push((text.to_owned(), style, self.source_line, self.link));
        }
    }

    /// Blank line between top-level blocks, never before the first.
    fn separate(&mut self) {
        self.finish();
        if self.lists.is_empty() && !self.blocks.is_empty() {
            self.blocks.push(Block {
                prefix: String::new(),
                indent: 0,
                spans: Vec::new(),
                source_line: self.source_line.saturating_sub(1),
            });
        }
    }

    /// Shows a construct exactly as written, one block per source line.
    fn source(&mut self, range: std::ops::Range<usize>) {
        self.finish();
        let raw = self.source[range.clone()].trim_end_matches('\n');
        for (offset, line) in raw.split('\n').enumerate() {
            self.blocks.push(Block {
                prefix: String::new(),
                indent: 0,
                spans: vec![(
                    line.to_owned(),
                    Style::new(),
                    self.source_line + offset,
                    None,
                )],
                source_line: self.source_line + offset,
            });
        }
        self.skip_until = range.end;
    }

    fn event(&mut self, event: Event<'_>, range: std::ops::Range<usize>) {
        if range.start < self.skip_until {
            return;
        }
        self.source_line = self
            .starts
            .partition_point(|start| *start <= range.start)
            .saturating_sub(1);
        match event {
            Event::Start(
                Tag::Table(_)
                | Tag::HtmlBlock
                | Tag::CodeBlock(_)
                | Tag::BlockQuote(_)
                | Tag::FootnoteDefinition(_)
                | Tag::MetadataBlock(_)
                | Tag::DefinitionList,
            ) => {
                self.separate();
                self.source(range);
            }
            Event::Start(Tag::Image { .. }) => {
                let text = self.source[range.clone()].to_owned();
                self.push(&text, Style::new());
                self.skip_until = range.end;
            }
            Event::Start(Tag::Heading { .. }) => {
                self.separate();
                self.heading = true;
                self.start(String::new(), 0);
            }
            Event::End(TagEnd::Heading(_)) => {
                self.heading = false;
                self.finish();
            }
            Event::Start(Tag::Paragraph) => {
                if self.current.is_none() {
                    self.separate();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if self.lists.is_empty() {
                    self.finish();
                }
            }
            Event::Start(Tag::List(start)) => {
                if self.lists.is_empty() {
                    self.separate();
                } else {
                    self.finish();
                }
                self.lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                self.finish();
                self.lists.pop();
            }
            Event::Start(Tag::Item) => {
                let depth = self.lists.len().saturating_sub(1);
                let marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}. ");
                        *number += 1;
                        marker
                    }
                    _ => "• ".to_owned(),
                };
                let indent = depth * 2 + marker.chars().count();
                self.start(format!("{}{marker}", " ".repeat(depth * 2)), indent);
            }
            Event::End(TagEnd::Item) => self.finish(),
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => {
                self.destination = Some(dest_url.to_string());
                if let Some(kind) = crate::links::classify(&dest_url, self.handlers) {
                    self.link = Some(self.links.len());
                    self.links.push(Link {
                        target: dest_url.to_string(),
                        kind,
                        source: self.source_line,
                        offset: range.start,
                    });
                }
            }
            Event::End(TagEnd::Link) => {
                self.source_line = self
                    .starts
                    .partition_point(|start| *start < range.end)
                    .saturating_sub(1);
                if let Some(url) = self.destination.take() {
                    self.push(&format!(" ({url})"), self.look.role(Role::Dim));
                }
                self.link = None;
            }
            Event::Text(text) => {
                let style = self.style();
                self.push(&text, style);
            }
            Event::Code(code) => self.push(
                &code,
                if self.link.is_some() {
                    self.style()
                } else {
                    self.look.role(Role::Accent)
                },
            ),
            Event::SoftBreak => self.push(" ", self.style()),
            Event::HardBreak => {
                let indent = self.current.as_ref().map_or(0, |block| block.indent);
                self.start(" ".repeat(indent), indent);
            }
            Event::Rule => {
                self.separate();
                self.blocks.push(Block {
                    prefix: String::new(),
                    indent: 0,
                    spans: vec![(
                        "───".into(),
                        self.look.role(Role::Dim),
                        self.source_line,
                        None,
                    )],
                    source_line: self.source_line,
                });
            }
            // Inline HTML, footnote references and anything else: as written.
            Event::Html(text) | Event::InlineHtml(text) => self.push(&text, Style::new()),
            _ => {
                let text = self.source[range].to_owned();
                self.push(&text, Style::new());
            }
        }
    }
}

/// Renders sanitized notes as styled lines wrapped to `width` display cells.
pub fn render(text: &str, width: usize, look: crate::look::Look) -> Vec<Line<'static>> {
    render_mapped(text, width, look).lines
}

/// Painted lines and their zero-based notebook source lines.
#[derive(Debug, Clone)]
pub(super) struct Link {
    pub target: String,
    pub kind: crate::links::Kind,
    pub source: usize,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub(super) struct LinkHit {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub link: usize,
}

pub(super) struct Mapped {
    pub lines: Vec<Line<'static>>,
    pub sources: Vec<usize>,
    pub links: Vec<Link>,
    pub hits: Vec<LinkHit>,
}

pub(super) fn render_mapped(text: &str, width: usize, look: crate::look::Look) -> Mapped {
    render_links(text, width, look, &crate::links::Handlers::new())
}

pub(super) fn render_links(
    text: &str,
    width: usize,
    look: crate::look::Look,
    handlers: &crate::links::Handlers,
) -> Mapped {
    let starts = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let mut renderer = Renderer {
        source: text,
        starts,
        source_line: 0,
        look,
        blocks: Vec::new(),
        current: None,
        bold: 0,
        italic: 0,
        heading: false,
        link: None,
        destination: None,
        links: Vec::new(),
        handlers,
        lists: Vec::new(),
        skip_until: 0,
    };
    let options = Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES;
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        renderer.event(event, range);
    }
    renderer.finish();
    let mut mapped = Mapped {
        lines: Vec::new(),
        sources: Vec::new(),
        links: renderer.links,
        hits: Vec::new(),
    };
    for block in renderer.blocks {
        let (lines, mut hits) = wrap(block, width.max(1));
        for hit in &mut hits {
            hit.line += mapped.lines.len();
        }
        for (line, source) in lines {
            mapped.lines.push(line);
            mapped.sources.push(source);
        }
        mapped.hits.extend(hits);
    }
    mapped
}

fn cells(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Greedy word wrap over styled spans; continuation lines use the hanging
/// indent, and a word longer than a line is split between characters.
fn wrap(block: Block, width: usize) -> (Vec<(Line<'static>, usize)>, Vec<LinkHit>) {
    let mut hits: Vec<LinkHit> = Vec::new();
    let mut lines = Vec::new();
    let mut line: Vec<Span<'static>> = vec![Span::raw(block.prefix.clone())];
    let mut used = cells(&block.prefix);
    let start = used;
    let mut source_line = block.source_line;
    let indent = " ".repeat(block.indent);
    let mut words: Vec<(String, Style, usize, Option<usize>)> = Vec::new();
    for (text, style, source, link) in block.spans {
        for word in text.split_inclusive(' ') {
            words.push((word.to_owned(), style, source, link));
        }
    }
    for (word, style, source, link) in words {
        let body = word.trim_end_matches(' ');
        if used > start && used + cells(body) > width {
            lines.push((Line::from(std::mem::take(&mut line)), source_line));
            source_line = source;
            line.push(Span::raw(indent.clone()));
            used = block.indent;
        }
        let mut piece = String::new();
        for character in word.chars() {
            let size = character.width().unwrap_or(0);
            if used + size > width {
                if character == ' ' {
                    continue;
                }
                line.push(Span::styled(std::mem::take(&mut piece), style));
                lines.push((Line::from(std::mem::take(&mut line)), source_line));
                source_line = source;
                line.push(Span::raw(indent.clone()));
                used = block.indent;
            }
            if let Some(link) = link.filter(|_| size > 0) {
                if let Some(last) = hits
                    .last_mut()
                    .filter(|h| h.line == lines.len() && h.link == link && h.end == used)
                {
                    last.end += size;
                } else {
                    hits.push(LinkHit {
                        line: lines.len(),
                        start: used,
                        end: used + size,
                        link,
                    });
                }
            }
            piece.push(character);
            used += size;
        }
        line.push(Span::styled(piece, style));
    }
    lines.push((Line::from(line), source_line));
    (lines, hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::Look;

    #[test]
    fn mapped_render_keeps_source_lines_through_wrapping_and_unsupported_blocks() {
        let source = "# Title\n\n- wide words that wrap\n- next\n\n```\ncode\n```";
        let mapped = render_mapped(source, 12, Look::default());
        assert_eq!(mapped.lines, render(source, 12, Look::default()));
        assert_eq!(mapped.lines.len(), mapped.sources.len());
        assert_eq!(mapped.sources[0], 0);
        assert_eq!(
            mapped.sources[1], 1,
            "separator is not the following list item"
        );
        assert!(mapped.sources.iter().filter(|source| **source == 2).count() > 1);
        let code = text(&mapped.lines)
            .iter()
            .position(|line| line == "code")
            .unwrap();
        assert_eq!(mapped.sources[code], 6);
        assert!(
            mapped
                .sources
                .iter()
                .all(|line| *line < source.split('\n').count())
        );
        let linked = render_mapped("[alpha\nbeta](https://example.com)", 9, Look::default());
        assert!(
            linked.sources.windows(2).all(|pair| pair[0] <= pair[1]),
            "a multiline link target belongs to its closing source line"
        );
    }

    #[test]
    fn link_cells_follow_unicode_wrap_and_undefined_schemes_stay_plain() {
        let mapped = render_links(
            "- [界 **wide** `code`](tmt:jump/auth-fix) [evil](javascript:x) #412",
            14,
            Look::default(),
            &crate::links::Handlers::new(),
        );
        assert_eq!(mapped.links.len(), 1);
        assert_eq!(mapped.links[0].target, "tmt:jump/auth-fix");
        assert!(mapped.hits.iter().any(|hit| hit.line > 0));
        for hit in &mapped.hits {
            assert!(hit.start >= 2 && hit.end <= 14 && hit.start < hit.end);
            assert_eq!(hit.link, 0);
        }
        assert!(
            mapped
                .lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content.contains("界")
                    && span.style.add_modifier.contains(Modifier::UNDERLINED))
        );
        let plain = render_mapped("[evil](javascript:x)", 80, Look::default());
        assert!(plain.hits.is_empty());
        assert!(
            plain
                .lines
                .iter()
                .flat_map(|line| &line.spans)
                .all(|span| !span.style.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .map(|line| line.trim_end().to_owned())
            .collect()
    }

    fn style_of(lines: &[Line], needle: &str) -> Style {
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.contains(needle))
            .map(|span| span.style)
            .unwrap_or_else(|| panic!("{needle} not rendered"))
    }

    #[test]
    fn the_supported_subset_is_styled_and_nested_lists_indent() {
        let notes = "## Now\n\n- tokens: **waiting** on Ben\n  - *login* vs `sweep`\n- see [PR 412](https://x/412)\n\n1. first\n2. second\n";
        let lines = render(notes, 60, Look::default());
        assert_eq!(
            text(&lines),
            [
                "Now",
                "",
                "• tokens: waiting on Ben",
                "  • login vs sweep",
                "• see PR 412 (https://x/412)",
                "",
                "1. first",
                "2. second",
            ]
        );
        assert!(
            style_of(&lines, "Now")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            style_of(&lines, "waiting")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            style_of(&lines, "login")
                .add_modifier
                .contains(Modifier::ITALIC)
        );
        assert_eq!(
            style_of(&lines, "sweep").fg,
            Look::default().role(Role::Accent).fg
        );
        assert!(
            style_of(&lines, "412")
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn unsupported_blocks_are_shown_as_their_source() {
        let notes = "| a | b |\n|---|---|\n| 1 | 2 |\n\n<div>raw html</div>\n\n```sh\ntmt x\n```\n\n> quoted\n\nsee ![diagram](d.png) and <b>bold</b>[^1]\n\n[^1]: footnote\n";
        let rendered = text(&render(notes, 80, Look::default()));
        for expected in [
            "| a | b |",
            "|---|---|",
            "| 1 | 2 |",
            "<div>raw html</div>",
            "```sh",
            "tmt x",
            "```",
            "> quoted",
            "[^1]: footnote",
        ] {
            assert!(
                rendered.iter().any(|line| line == expected),
                "{expected}: {rendered:#?}"
            );
        }
        let inline = rendered
            .iter()
            .find(|line| line.starts_with("see "))
            .unwrap();
        assert!(inline.contains("![diagram](d.png)") && inline.contains("<b>bold</b>"));
        assert!(inline.contains("[^1]"), "{inline}");
    }

    #[test]
    fn wrapping_respects_width_hanging_indent_and_wide_characters() {
        let notes = "- 安装指南 consolidates the install guide into one page with platform tabs\n";
        let lines = render(notes, 20, Look::default());
        for line in &lines {
            let width: usize = line.spans.iter().map(|span| cells(&span.content)).sum();
            assert!(width <= 20, "{line:?}");
        }
        let rendered = text(&lines);
        assert!(rendered[0].starts_with("• 安装指南"));
        assert!(
            rendered[1..].iter().all(|line| line.starts_with("  ")),
            "{rendered:#?}"
        );
        let long = text(&render(&"x".repeat(45), 20, Look::default()));
        assert_eq!(long, ["x".repeat(20), "x".repeat(20), "x".repeat(5)]);
    }

    #[test]
    fn hostile_notes_render_without_any_control_or_format_character() {
        let hostile =
            "# \u{1b}]0;PWNED\u{7}Title\n\n- \u{1b}[2J**bold**\u{202e}rev\n\n`\u{9b}31mcode`";
        let lines = render(&crate::board::notes::sanitize(hostile), 40, Look::default());
        let all: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
            .collect();
        assert!(
            !all.chars()
                .any(|c| c.is_control() || ('\u{202a}'..='\u{202e}').contains(&c)),
            "{all:?}"
        );
        assert!(
            all.contains("Title")
                && all.contains("bold")
                && all.contains("rev")
                && all.contains("code")
        );
    }
}
