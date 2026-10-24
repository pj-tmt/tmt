//! Copying a selection of the transcript as the Markdown source of its messages (#4457).
//! The transcript keeps, for each displayed line, the message it came from and the source
//! line within it; a selection maps back to those source lines instead of the painted text.
//! Tables, code, headings, lists and quotes are copied as whole source lines. A selection
//! that starts or ends inside a plain paragraph line copies the exact source characters.
use crate::board::member_links::{Link, Origin};

/// Where one displayed transcript line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct At {
    pub message: usize,
    pub line: usize,
}

/// The source of every message in a transcript and the origin of each displayed line.
#[derive(Default)]
pub(super) struct Map {
    pub messages: Vec<String>,
    pub rows: Vec<Row>,
}

pub(super) struct Row {
    pub at: Option<At>,
    pub shown: String,
    origins: Vec<Origin>,
}

impl Row {
    pub fn new(at: Option<At>, shown: String, links: &[Link]) -> Self {
        Self {
            at,
            shown,
            origins: links
                .iter()
                .filter_map(|link| link.source.clone())
                .collect(),
        }
    }
}

/// One end of a selection: a byte offset in the painted text of its row.
pub(super) struct End<'a> {
    pub row: &'a str,
    pub at: usize,
}

/// A selection of displayed lines `first..=last`.
pub(super) struct Selected<'a> {
    pub first: usize,
    pub last: usize,
    /// What each line displays inside the selection, used for lines that carry no source.
    pub shown: &'a [String],
    /// Its first selected character, in the painted row `first`.
    pub start: End<'a>,
    /// Just after its last selected character, in the painted row `last`.
    pub end: End<'a>,
}

/// The text to copy for a selection.
pub(super) fn passage(map: &Map, selected: &Selected) -> String {
    let (first, last) = (selected.first, selected.last);
    let mut parts: Vec<String> = Vec::new();
    let mut line = first;
    while line <= last {
        let Some(start) = map.rows.get(line).and_then(|row| row.at) else {
            parts.push(
                selected
                    .shown
                    .get(line - first)
                    .cloned()
                    .unwrap_or_default(),
            );
            line += 1;
            continue;
        };
        // A run of displayed lines of one message is copied as the source lines it covers.
        let (mut low, mut high, mut end) = (start.line, start.line, line);
        while end < last
            && let Some(next) = map.rows.get(end + 1).and_then(|row| row.at)
            && next.message == start.message
        {
            low = low.min(next.line);
            high = high.max(next.line);
            end += 1;
        }
        let text = &map.messages[start.message];
        let cut = Cut {
            head: (line == first)
                .then(|| exact(map, text, line, &selected.start, true))
                .flatten(),
            tail: (end == last)
                .then(|| exact(map, text, end, &selected.end, false))
                .flatten(),
        };
        parts.push(source(text, low, high, &cut));
        line = end + 1;
    }
    parts.join("\n")
}

/// Where inside its first and last source line a copy starts and stops, when those ends
/// lie in plain paragraph text.
#[derive(Default)]
struct Cut {
    head: Option<usize>,
    tail: Option<usize>,
}

/// A block of a message that is only valid Markdown with its opening lines.
enum Block {
    /// A fenced code block: its opening fence line and, when it has one, its closing fence.
    Fence { open: usize, close: Option<usize> },
    /// A table: its header row, whose delimiter row follows.
    Table { head: usize, end: usize },
    /// A paragraph outside any list or quote: its source lines.
    Prose { start: usize, end: usize },
}

/// The source lines `low..=high` of `text`, with the lines a partial block needs to stay
/// valid: the fences around selected code and the header of a selected table body.
fn source(text: &str, low: usize, high: usize, cut: &Cut) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let high = high.min(lines.len().saturating_sub(1));
    let low = low.min(high);
    let blocks = blocks(text, &lines);
    let mut out: Vec<String> = Vec::new();
    for block in &blocks {
        match block {
            Block::Fence { open, close } => {
                let end = close.unwrap_or(lines.len());
                if *open < low && low < end {
                    out.push(lines[*open].to_owned());
                }
            }
            Block::Table { head, end } => {
                if head + 1 < low && low <= *end {
                    out.push(lines[*head].to_owned());
                    out.push(lines[*head + 1].to_owned());
                }
            }
            Block::Prose { .. } => {}
        }
    }
    let prose = |line: usize| {
        blocks.iter().any(
            |block| matches!(block, Block::Prose { start, end } if (*start..=*end).contains(&line)),
        )
    };
    for (line, text) in lines.iter().enumerate().take(high + 1).skip(low) {
        let mut text = *text;
        let hard_break = line + 1 < lines.len() && text.ends_with("  ");
        let from = if line == low && prose(line) {
            cut.head.filter(|at| text.is_char_boundary(*at))
        } else {
            None
        };
        let to = if line == high && prose(line) {
            cut.tail.filter(|at| text.is_char_boundary(*at))
        } else {
            None
        };
        let start = from.unwrap_or(0);
        let to = to.map(|to| closing(text, start, to.max(start)));
        if let Some(to) = to {
            text = &text[..to.min(text.len())];
        }
        text = &text[start.min(text.len())..];
        out.push(if to.is_some() && hard_break {
            text.trim_end_matches(' ').to_owned()
        } else {
            text.to_owned()
        });
    }
    for block in &blocks {
        if let Block::Fence {
            open,
            close: Some(close),
        } = block
            && open < &high
            && high < *close
        {
            out.push(lines[*close].to_owned());
        }
    }
    out.join("\n")
}

/// The byte offset in the source line behind displayed line `row` of the selection end at
/// byte `at` of that row: the start of the line when the end is its first displayed
/// character (so leading markers come along), the end of the line when it is the last, else
/// the exact character. `None` when the line is not plain prose or its text cannot be aligned
/// with its source.
fn exact(map: &Map, text: &str, row: usize, end: &End, head: bool) -> Option<usize> {
    let source_line = map.rows.get(row)?.at?;
    let lines: Vec<&str> = text.split('\n').collect();
    let line = *lines.get(source_line.line)?;
    let line_start = lines
        .iter()
        .take(source_line.line)
        .map(|line| line.len() + 1)
        .sum::<usize>();
    // Spaces are not anchors, and the painted row may carry a pane border or a scroll bar
    // beside the displayed text: find that text inside it.
    let nonspace = |text: &str| {
        text.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
    };
    let painted = nonspace(end.row);
    let lead = painted
        .find(&nonspace(&map.rows.get(row)?.shown))
        .map(|byte| painted[..byte].chars().count())?;
    let ordinal = nonspace(&end.row[..end.at.min(end.row.len())])
        .chars()
        .count()
        .saturating_sub(lead);
    // The displayed rows of this source line.
    let same = |index: usize| map.rows.get(index).and_then(|row| row.at) == Some(source_line);
    let mut top = row;
    while top > 0 && same(top - 1) {
        top -= 1;
    }
    let mut bottom = row;
    while same(bottom + 1) {
        bottom += 1;
    }
    // Align the displayed characters with the source characters: the display is the source
    // without its markers.
    let mut source_chars = line
        .char_indices()
        .filter(|(_, c)| !c.is_whitespace())
        .peekable();
    let mut shown: Vec<(usize, usize, usize, usize)> = Vec::new(); // row, nth, from, to
    for index in top..=bottom {
        let row = map.rows.get(index)?;
        let mut origins = row.origins.iter().peekable();
        for (nth, (byte, character)) in row
            .shown
            .char_indices()
            .filter(|(_, c)| !c.is_whitespace())
            .enumerate()
        {
            while origins
                .peek()
                .is_some_and(|origin| origin.shown.end <= byte)
            {
                origins.next();
            }
            if let Some(origin) = origins.peek().filter(|origin| origin.shown.contains(&byte)) {
                let from = origin.source.start.checked_sub(line_start)?;
                let to = origin.source.end.checked_sub(line_start)?;
                if to > line.len() {
                    return None;
                }
                while source_chars.peek().is_some_and(|(at, _)| *at < to) {
                    source_chars.next();
                }
                shown.push((index, nth, from, to));
            } else {
                let (from, matched) = source_chars.find(|(_, c)| *c == character)?;
                shown.push((index, nth, from, from + matched.len_utf8()));
            }
        }
    }
    let first_char = shown.first()?;
    let last_char = shown.last()?;
    if head {
        let entry = shown
            .iter()
            .find(|(r, nth, ..)| (*r, *nth) >= (row, ordinal))?;
        Some(if entry == first_char { 0 } else { entry.2 })
    } else {
        let entry = shown
            .iter()
            .rev()
            .find(|(r, nth, ..)| (*r, *nth) < (row, ordinal))?;
        if entry == last_char {
            return Some(line.len());
        }
        Some(entry.3)
    }
}

/// `end` moved past the closing marker of an emphasis, code span or strike that opens
/// inside the copied text (from `start`) and closes right after it.
fn closing(line: &str, start: usize, end: usize) -> usize {
    if end >= line.len() || !line.is_char_boundary(end) {
        return end;
    }
    let rest = &line[end..];
    let run: String = rest
        .chars()
        .take_while(|c| matches!(c, '*' | '_' | '~' | '`'))
        .collect();
    if run.is_empty() {
        return end;
    }
    // An odd count of the marker inside the copied text leaves one opener to close.
    let opened = line[start..end].matches(run.as_str()).count();
    if opened % 2 == 1 {
        end + run.len()
    } else {
        end
    }
}

fn blocks(text: &str, lines: &[&str]) -> Vec<Block> {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let line_of = |at: usize| {
        starts
            .partition_point(|start| *start <= at)
            .saturating_sub(1)
    };
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    for (event, range) in
        Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES).into_offset_iter()
    {
        let last = line_of(range.end.saturating_sub(1).max(range.start));
        match event {
            Event::Start(Tag::List(_) | Tag::BlockQuote(_)) => depth += 1,
            Event::End(TagEnd::List(_) | TagEnd::BlockQuote(_)) => depth = depth.saturating_sub(1),
            Event::Start(Tag::Paragraph) if depth == 0 => blocks.push(Block::Prose {
                start: line_of(range.start),
                end: last,
            }),
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                let open = line_of(range.start);
                let closes = last > open
                    && lines.get(last).is_some_and(|line| {
                        let line = line.trim_start();
                        line.starts_with("```") || line.starts_with("~~~")
                    });
                blocks.push(Block::Fence {
                    open,
                    close: closes.then_some(last),
                });
            }
            Event::Start(Tag::Table(_)) => blocks.push(Block::Table {
                head: line_of(range.start),
                end: last,
            }),
            _ => {}
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(messages: &[&str], lines: &[Option<(usize, usize)>]) -> Map {
        Map {
            messages: messages.iter().map(|text| (*text).to_owned()).collect(),
            rows: lines
                .iter()
                .map(|at| {
                    Row::new(
                        at.map(|(message, line)| At { message, line }),
                        String::new(),
                        &[],
                    )
                })
                .collect(),
        }
    }

    /// A map whose displayed rows are known, so a paragraph's ends can be cut exactly.
    fn shown_map(messages: &[&str], lines: &[Option<(usize, usize)>], rows: &[&str]) -> Map {
        let mut map = map(messages, lines);
        for (line, shown) in map.rows.iter_mut().zip(rows) {
            line.shown = (*shown).to_owned();
        }
        map
    }

    fn passage(map: &Map, shown: &[String], first: usize, last: usize) -> String {
        super::passage(
            map,
            &Selected {
                first,
                last,
                shown,
                start: End { row: "", at: 0 },
                end: End { row: "", at: 0 },
            },
        )
    }

    /// Selects the bytes `start..end` of row `first` to row `last`.
    fn cut(map: &Map, first: usize, last: usize, start: usize, end: usize) -> String {
        let shown: Vec<String> = (first..=last).map(|_| String::new()).collect();
        super::passage(
            map,
            &Selected {
                first,
                last,
                shown: &shown,
                start: End {
                    row: &map.rows[first].shown,
                    at: start,
                },
                end: End {
                    row: &map.rows[last].shown,
                    at: end,
                },
            },
        )
    }

    fn shown(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|row| (*row).to_owned()).collect()
    }

    fn rendered_map(source: &str, width: usize) -> Map {
        let rendered = crate::board::markdown::render_conversation_with_members(
            source,
            width,
            crate::look::Look::default(),
            None,
        );
        Map {
            messages: vec![source.to_owned()],
            rows: rendered
                .lines
                .iter()
                .zip(&rendered.sources)
                .zip(&rendered.mentions)
                .map(|((line, &source), links)| {
                    Row::new(
                        (line.width() > 0).then_some(At {
                            message: 0,
                            line: source,
                        }),
                        line.to_string(),
                        links,
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn shortened_captions_copy_the_whole_original_message_with_its_eof_spaces() {
        for source in [
            crate::board::markdown::reported::PATH.to_owned(),
            "before '/tmp/one file_name.txt' after ".to_owned(),
            "before '/tmp/one file_name.txt' after  ".to_owned(),
        ] {
            for width in [12, 24, 80, 200] {
                let map = rendered_map(&source, width);
                let last = map.rows.len() - 1;
                assert_eq!(
                    cut(&map, 0, last, 0, map.rows[last].shown.len()),
                    source,
                    "{width}"
                );
            }
        }
    }

    #[test]
    fn a_caption_selection_copies_its_raw_token_and_adjacent_prose_stays_exact() {
        let source = "before '/tmp/my_file name.txt' after";
        let map = rendered_map(source, 200);
        let row = &map.rows[0].shown;
        let file = row.find("my_file").unwrap();
        let after = row.find("after").unwrap();
        assert_eq!(cut(&map, 0, 0, 0, "before".len()), "before");
        assert_eq!(
            cut(&map, 0, 0, file + 1, file + 4),
            "'/tmp/my_file name.txt'"
        );
        assert_eq!(cut(&map, 0, 0, after, row.len()), "after");
        assert_eq!(
            cut(&map, 0, 0, 0, row.find(" after").unwrap()),
            "before '/tmp/my_file name.txt'"
        );
    }

    #[test]
    fn equal_basenames_copy_the_selected_occurrence_after_wrapping() {
        for width in [12, 200] {
            let map = rendered_map("/tmp/one.txt /other/one.txt", width);
            let row = map.rows.len() - 1;
            let start = map.rows[row].shown.rfind("one.txt").unwrap();
            assert_eq!(cut(&map, row, row, start + 1, start + 4), "/other/one.txt");
        }
    }

    #[test]
    fn a_painted_message_gutter_is_not_part_of_the_source_alignment() {
        let map = rendered_map("before '/tmp/a.txt' after", 200);
        let painted = crate::board::exchange_view::body(
            ratatui::text::Line::from(map.rows[0].shown.clone()),
            crate::look::Look::default(),
        )
        .to_string();
        let start = painted.find("before").unwrap();
        assert_eq!(
            super::passage(
                &map,
                &Selected {
                    first: 0,
                    last: 0,
                    shown: std::slice::from_ref(&painted),
                    start: End {
                        row: &painted,
                        at: start
                    },
                    end: End {
                        row: &painted,
                        at: start + "before".len()
                    },
                }
            ),
            "before"
        );
    }

    #[test]
    fn reported_quote_and_paths_copy_the_original_source_after_real_wrapping() {
        use crate::{
            board::markdown::{render_conversation_with_members, reported},
            look::Look,
        };

        for source in [
            reported::QUOTE,
            reported::PATH,
            "/tmp/some_dir/file_name.rs",
            "src/some_module/file_name.rs",
        ] {
            for width in [160, 100, 80, 60, 40, 24] {
                let rendered =
                    render_conversation_with_members(source, width, Look::default(), None);
                let rows: Vec<String> = rendered.lines.iter().map(ToString::to_string).collect();
                let map = Map {
                    messages: vec![source.to_owned()],
                    rows: rendered
                        .sources
                        .iter()
                        .zip(&rows)
                        .zip(&rendered.mentions)
                        .map(|((&line, shown), links)| {
                            Row::new(Some(At { message: 0, line }), shown.clone(), links)
                        })
                        .collect(),
                };
                assert_eq!(
                    passage(&map, &rows, 0, rows.len() - 1),
                    source,
                    "width={width}, source={source:?}"
                );
            }
        }
    }

    #[test]
    fn wrapped_prose_copies_its_unwrapped_source_line() {
        let map = map(
            &["a long paragraph that wraps\nnext line"],
            &[Some((0, 0)), Some((0, 0)), Some((0, 1))],
        );
        let rows = shown(&["a long paragraph", "that wraps", "next line"]);
        assert_eq!(
            passage(&map, &rows, 0, 2),
            "a long paragraph that wraps\nnext line"
        );
    }

    #[test]
    fn a_table_copies_its_pipe_rows_with_the_header_and_delimiter() {
        let source = "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
        let map = map(
            &[source],
            &[
                Some((0, 0)),
                Some((0, 0)),
                Some((0, 0)),
                Some((0, 2)),
                Some((0, 3)),
                Some((0, 3)),
            ],
        );
        let rows = shown(&["┌─┐", "│ a │ b │", "├─┤", "│ 1 │ 2 │", "│ 3 │ 4 │", "└─┘"]);
        assert_eq!(passage(&map, &rows, 0, 5), source);
        // Only the last row: the header and delimiter still come with it, so it pastes as a table.
        assert_eq!(
            passage(&map, &rows, 4, 4),
            "| a | b |\n|---|---|\n| 3 | 4 |"
        );
    }

    #[test]
    fn a_code_block_copies_its_fence_and_the_selected_lines() {
        let source = "```rust\nlet a = 1;\nlet b = 2;\nlet c = 3;\n```";
        let map = map(&[source], &[Some((0, 1)), Some((0, 2)), Some((0, 3))]);
        let rows = shown(&["let a = 1;", "let b = 2;", "let c = 3;"]);
        assert_eq!(passage(&map, &rows, 0, 2), source);
        assert_eq!(passage(&map, &rows, 1, 1), "```rust\nlet b = 2;\n```");
    }

    #[test]
    fn several_messages_copy_each_covered_source_in_order_separated_as_displayed() {
        let map = map(
            &["first **one**\nsecond line", "# Reply\n\n- item"],
            &[
                Some((0, 0)),
                Some((0, 1)),
                None,
                None,
                Some((1, 0)),
                None,
                Some((1, 2)),
            ],
        );
        let rows = shown(&[
            "first one",
            "second line",
            "",
            "alpha · now",
            "Reply",
            "",
            "• item",
        ]);
        assert_eq!(
            passage(&map, &rows, 0, 6),
            "first **one**\nsecond line\n\nalpha · now\n# Reply\n\n- item"
        );
    }

    #[test]
    fn lines_without_a_source_keep_their_displayed_text() {
        let map = map(&["body"], &[None, Some((0, 0))]);
        let rows = shown(&["alpha · now", "body"]);
        assert_eq!(passage(&map, &rows, 0, 1), "alpha · now\nbody");
    }

    #[test]
    fn a_selection_inside_a_paragraph_line_copies_the_exact_source_characters() {
        let source = "first **styled** passage with `code` and more";
        let row = "first styled passage with code and more";
        let map = shown_map(&[source], &[Some((0, 0))], &[row]);
        // The whole displayed line is the whole source line, markers and all.
        assert_eq!(cut(&map, 0, 0, 0, row.len()), source);
        // From the first character to inside the bold word: its closing marker comes along.
        assert_eq!(cut(&map, 0, 0, 0, "first styled".len()), "first **styled**");
        // Inside the line on both sides: exactly the characters, nothing added.
        let from = row.find("passage").unwrap();
        assert_eq!(cut(&map, 0, 0, from, from + "passage".len()), "passage");
        // A code span opened inside the selection and closed right after it is kept whole.
        let from = row.find("with").unwrap();
        assert_eq!(
            cut(&map, 0, 0, from, from + "with code".len()),
            "with `code`"
        );
        // A selection of the middle of the bold word copies only its letters.
        let from = row.find("styled").unwrap();
        assert_eq!(cut(&map, 0, 0, from + 1, from + 4), "tyl");
    }

    #[test]
    fn a_wrapped_paragraph_line_is_cut_across_its_rows_and_whole_lines_stay_whole() {
        let source = "alpha **beta gamma** delta epsilon\nsecond source line";
        let rows = ["alpha beta gamma", "delta epsilon", "second source line"];
        let map = shown_map(
            &[source],
            &[Some((0, 0)), Some((0, 0)), Some((0, 1))],
            &rows,
        );
        // From inside the first row to inside the second row of one source line.
        assert_eq!(cut(&map, 0, 1, 6, 5), "beta gamma** delta");
        // From the middle of the first source line through the end of the second: the
        // second is whole.
        assert_eq!(
            cut(&map, 1, 2, 0, rows[2].len()),
            "delta epsilon\nsecond source line"
        );
    }

    #[test]
    fn table_and_code_ends_never_cut_inside_a_source_line() {
        let source = "| a | b |\n|---|---|\n| 111 | 222 |";
        let map = shown_map(
            &[source],
            &[Some((0, 0)), Some((0, 0)), Some((0, 2))],
            &["│ a │ b │", "├───┼───┤", "│ 111 │ 222 │"],
        );
        assert_eq!(cut(&map, 2, 2, 4, 7), "| a | b |\n|---|---|\n| 111 | 222 |");
    }

    #[test]
    fn hard_break_paragraph_lines_cut_at_their_ends() {
        let source = "passage 000  \npassage 001  \npassage 002  \n";
        let rows = ["passage 000", "passage 001", "passage 002"];
        let map = shown_map(
            &[source],
            &[Some((0, 0)), Some((0, 1)), Some((0, 2))],
            &rows,
        );
        assert_eq!(
            cut(&map, 0, 2, 0, 11),
            "passage 000  \npassage 001  \npassage 002"
        );
        assert_eq!(cut(&map, 1, 1, 0, 11), "passage 001");
        assert_eq!(cut(&map, 1, 1, 3, 9), "sage 0");
    }
}
