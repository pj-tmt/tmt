//! A member pane's visible screen, read through `pane.screen` and drawn read-only. The
//! text is another program's output: only colours and text attributes survive parsing; any
//! other escape sequence is dropped, so a screen can never move the cursor, retitle the
//! terminal or write to the clipboard.
use crate::look::Look;
use ratatui::style::{Color, Modifier, Style};
use tmt_cli_style::Depth;
use unicode_width::UnicodeWidthChar;

/// A colour as the screen named it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paint {
    /// 0–15 are the terminal's own colours; 16–255 the xterm cube and grays.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Attr {
    pub fg: Option<Paint>,
    pub bg: Option<Paint>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

/// Text sharing one set of attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub text: String,
    pub attr: Attr,
}

pub(crate) type Row = Vec<Run>;

/// What the pane read last said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Observation {
    /// Nothing read yet.
    #[default]
    Pending,
    /// The identity has no pane, or the pane is gone.
    Gone,
    /// The host could not give the screen, with the reason it gave.
    Unavailable(String),
    Screen(Screen),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Screen {
    pub rows: Vec<Row>,
    /// Cells wide the widest row is.
    pub width: usize,
    pub read_ms: u64,
}

/// One `pane.screen` read of `identity`. A pane that is gone and a screen the host would not
/// give are told apart; a malformed answer counts as the second.
pub(super) fn read(core: &crate::core::Core, identity: &str) -> Observation {
    if !super::recent_responses::canonical_identity(identity) {
        return Observation::Unavailable("not an agent identity".into());
    }
    match core.api_within(
        "pane.screen",
        serde_json::json!({"identityId": identity}),
        std::time::Duration::from_secs(3),
    ) {
        Ok(value) => from_value(identity, &value, crate::status::now_ms()),
        Err(error) => match error.code.as_str() {
            "PANE_NOT_FOUND" | "NAME_NOT_FOUND" => Observation::Gone,
            _ => Observation::Unavailable(super::notes::single_line(&error.message)),
        },
    }
}

fn from_value(identity: &str, value: &serde_json::Value, read_ms: u64) -> Observation {
    let screen = (|| {
        if value["identityId"].as_str()? != identity
            || !matches!(value["format"].as_str()?, "ansi" | "plain")
        {
            return None;
        }
        let rows = parse(value["text"].as_str()?);
        let width = rows.iter().map(width).max().unwrap_or(0);
        Some(Screen {
            rows,
            width,
            read_ms,
        })
    })();
    screen.map_or_else(
        || Observation::Unavailable("the answer was not a pane screen".into()),
        Observation::Screen,
    )
}

const MAX_PARAMS: usize = 32;

/// Rows of `text`. `ansi` screens carry SGR; `plain` ones carry none, and any escape left in
/// them is dropped just the same.
pub(crate) fn parse(text: &str) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    let mut row: Row = Vec::new();
    let mut attr = Attr::default();
    let mut word = String::new();
    let mut chars = text.chars().peekable();
    let flush = |word: &mut String, row: &mut Row, attr: Attr| {
        if !word.is_empty() {
            row.push(Run {
                text: std::mem::take(word),
                attr,
            });
        }
    };
    while let Some(character) = chars.next() {
        match character {
            '\n' => {
                flush(&mut word, &mut row, attr);
                rows.push(std::mem::take(&mut row));
            }
            '\t' => word.push(' '),
            '\u{1b}' | '\u{9b}' => match if character == '\u{9b}' {
                Some('[')
            } else {
                chars.next()
            } {
                Some('[') => {
                    let mut params = String::new();
                    let mut final_byte = None;
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            final_byte = Some(next);
                            break;
                        }
                        if params.len() < 256 {
                            params.push(next);
                        }
                    }
                    if final_byte == Some('m') {
                        flush(&mut word, &mut row, attr);
                        apply(&mut attr, &params);
                    }
                }
                // String sequences end at BEL or ST and carry nothing we draw.
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' {
                            chars.next_if_eq(&'\\');
                            break;
                        }
                    }
                }
                // A two-byte escape, with an intermediate byte when it has one.
                Some(next) if ('\u{20}'..='\u{2f}').contains(&next) => {
                    chars.next();
                }
                _ => {}
            },
            other if other.is_control() => {}
            other => word.push(other),
        }
    }
    flush(&mut word, &mut row, attr);
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

fn apply(attr: &mut Attr, params: &str) {
    if params
        .bytes()
        .any(|b| !(b.is_ascii_digit() || b == b';' || b == b':'))
    {
        return;
    }
    let groups: Vec<&str> = params.split(';').take(MAX_PARAMS).collect();
    if groups.iter().all(|group| group.is_empty()) {
        *attr = Attr::default();
        return;
    }
    let number = |text: &str| text.parse::<u16>().ok();
    let mut index = 0;
    while index < groups.len() {
        let sub: Vec<&str> = groups[index].split(':').collect();
        let code = number(sub[0]).unwrap_or(0);
        index += 1;
        match code {
            0 => *attr = Attr::default(),
            1 => attr.bold = true,
            2 => attr.dim = true,
            3 => attr.italic = true,
            4 => attr.underline = true,
            7 => attr.reverse = true,
            22 => {
                attr.bold = false;
                attr.dim = false;
            }
            23 => attr.italic = false,
            24 => attr.underline = false,
            27 => attr.reverse = false,
            30..=37 => attr.fg = Some(Paint::Indexed((code - 30) as u8)),
            39 => attr.fg = None,
            40..=47 => attr.bg = Some(Paint::Indexed((code - 40) as u8)),
            49 => attr.bg = None,
            90..=97 => attr.fg = Some(Paint::Indexed((code - 90 + 8) as u8)),
            100..=107 => attr.bg = Some(Paint::Indexed((code - 100 + 8) as u8)),
            38 | 48 => {
                // `38;5;n` and `38;2;r;g;b`, or the colon form `38:2::r:g:b`.
                let values: Vec<u16> = if sub.len() > 1 {
                    sub[1..].iter().filter_map(|part| number(part)).collect()
                } else {
                    let take = match groups.get(index).and_then(|g| number(g)) {
                        Some(5) => 2,
                        Some(2) => 4,
                        _ => 0,
                    };
                    let values = groups[index..(index + take).min(groups.len())]
                        .iter()
                        .filter_map(|part| number(part))
                        .collect();
                    index += take;
                    values
                };
                let paint = match values.as_slice() {
                    [5, n, ..] if *n <= 255 => Some(Paint::Indexed(*n as u8)),
                    [2, r, g, b, ..] | [2, _, r, g, b] if *r <= 255 && *g <= 255 && *b <= 255 => {
                        Some(Paint::Rgb(*r as u8, *g as u8, *b as u8))
                    }
                    _ => None,
                };
                if let Some(paint) = paint {
                    if code == 38 {
                        attr.fg = Some(paint);
                    } else {
                        attr.bg = Some(paint);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The 16 colours of the xterm palette, for the nearest-colour map at ANSI16.
const BASE16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

fn rgb(paint: Paint) -> (u8, u8, u8) {
    match paint {
        Paint::Rgb(r, g, b) => (r, g, b),
        Paint::Indexed(n) if n < 16 => BASE16[usize::from(n)],
        Paint::Indexed(n) if n < 232 => {
            let n = n - 16;
            let level = |step: u8| if step == 0 { 0 } else { 55 + 40 * step };
            (level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        Paint::Indexed(n) => {
            let gray = 8 + 10 * (n - 232);
            (gray, gray, gray)
        }
    }
}

fn nearest16(paint: Paint) -> u8 {
    if let Paint::Indexed(n) = paint
        && n < 16
    {
        return n;
    }
    let (r, g, b) = rgb(paint);
    let distance = |(pr, pg, pb): (u8, u8, u8)| {
        let d = |a: u8, b: u8| u32::from(a.abs_diff(b)).pow(2);
        d(r, pr) + d(g, pg) + d(b, pb)
    };
    (0..16u8)
        .min_by_key(|n| distance(BASE16[usize::from(*n)]))
        .expect("sixteen colours")
}

fn color(paint: Paint, depth: Depth) -> Option<Color> {
    match depth {
        Depth::None => None,
        Depth::Ansi16 => Some(Color::Indexed(nearest16(paint))),
        _ => Some(match paint {
            Paint::Indexed(n) => Color::Indexed(n),
            Paint::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }),
    }
}

/// The style of `attr` on this terminal: colours as far as its depth allows, attributes
/// always.
pub(crate) fn style(attr: Attr, look: Look) -> Style {
    let mut style = Style::new();
    if let Some(paint) = attr.fg
        && let Some(fg) = color(paint, look.depth)
    {
        style = style.fg(fg);
    }
    if let Some(paint) = attr.bg
        && let Some(bg) = color(paint, look.depth)
    {
        style = style.bg(bg);
    }
    for (on, modifier) in [
        (attr.bold, Modifier::BOLD),
        (attr.dim, Modifier::DIM),
        (attr.italic, Modifier::ITALIC),
        (attr.underline, Modifier::UNDERLINED),
        (attr.reverse, Modifier::REVERSED),
    ] {
        if on {
            style = style.add_modifier(modifier);
        }
    }
    style
}

/// Cells a row takes.
pub(crate) fn width(row: &Row) -> usize {
    row.iter()
        .flat_map(|run| run.text.chars())
        .map(|c| c.width().unwrap_or(0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::Look;
    use tmt_cli_style::{Theme, theme::Base};

    fn flat(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| row.iter().map(|run| run.text.as_str()).collect())
            .collect()
    }
    fn look(depth: Depth) -> Look {
        Look {
            theme: Theme::new(Base::Tmt),
            depth,
        }
    }

    #[test]
    fn colours_and_attributes_split_a_row_into_runs() {
        let rows = parse("\u{1b}[31mred\u{1b}[39m plain \u{1b}[1;4mbold\u{1b}[0m");
        assert_eq!(flat(&rows), ["red plain bold"]);
        let attrs: Vec<_> = rows[0].iter().map(|run| run.attr).collect();
        assert_eq!(attrs[0].fg, Some(Paint::Indexed(1)));
        assert_eq!(attrs[1], Attr::default());
        assert!(attrs[2].bold && attrs[2].underline);
    }

    #[test]
    fn extended_colours_in_both_forms() {
        let rows =
            parse("\u{1b}[38;5;208ma\u{1b}[48;2;1;2;3mb\u{1b}[38:2::9:8:7mc\u{1b}[48:5:21md");
        let attrs: Vec<_> = rows[0].iter().map(|run| run.attr).collect();
        assert_eq!(attrs[0].fg, Some(Paint::Indexed(208)));
        assert_eq!(attrs[1].bg, Some(Paint::Rgb(1, 2, 3)));
        assert_eq!(attrs[2].fg, Some(Paint::Rgb(9, 8, 7)));
        assert_eq!(attrs[3].bg, Some(Paint::Indexed(21)));
        // Truncated or out-of-range colours leave the attribute alone.
        let rows = parse("\u{1b}[38;5;999mx\u{1b}[38;2;1mY\u{1b}[38;5m");
        assert_eq!(flat(&rows), ["xY"]);
        assert!(rows[0].iter().all(|run| run.attr == Attr::default()));
    }

    #[test]
    fn every_other_escape_is_dropped_and_none_reaches_the_text() {
        let hostile = "a\u{1b}[2J\u{1b}[H\u{1b}[31;999Hb\u{1b}]0;title\u{7}c\u{1b}]52;c;AAAA\u{1b}\\d\
            \u{1b}Pq data\u{1b}\\e\u{1b}(0f\u{1b}7g\u{1b}=h\r\u{8}\u{7}i\u{9b}31mj";
        let rows = parse(hostile);
        assert_eq!(flat(&rows), ["abcdefghij"]);
        // Only the colour of the final C1 form survives, and only on its own text.
        let (last, rest) = rows[0].split_last().unwrap();
        assert!(rest.iter().all(|run| run.attr == Attr::default()));
        assert_eq!(
            (last.text.as_str(), last.attr.fg),
            ("j", Some(Paint::Indexed(1)))
        );
    }

    #[test]
    fn newlines_make_rows_tabs_become_a_space_and_an_unfinished_escape_ends_quietly() {
        assert_eq!(flat(&parse("one\ntwo\n\nfour")), ["one", "two", "", "four"]);
        assert_eq!(flat(&parse("a\tb")), ["a b"]);
        assert_eq!(flat(&parse("ok\u{1b}[31")), ["ok"]);
        assert_eq!(flat(&parse("ok\u{1b}")), ["ok"]);
        assert_eq!(flat(&parse("")), Vec::<String>::new());
    }

    #[test]
    fn a_reset_clears_everything_and_an_empty_sgr_is_a_reset() {
        let rows = parse("\u{1b}[1;31mA\u{1b}[mB\u{1b}[7mC\u{1b}[27mD");
        let attrs: Vec<_> = rows[0].iter().map(|run| run.attr).collect();
        assert!(attrs[0].bold);
        assert_eq!(attrs[1], Attr::default());
        assert!(attrs[2].reverse);
        assert!(!attrs[3].reverse);
    }

    #[test]
    fn colours_follow_the_terminal_depth_and_attributes_always_stay() {
        let attr = Attr {
            fg: Some(Paint::Rgb(250, 10, 10)),
            bg: Some(Paint::Indexed(21)),
            bold: true,
            reverse: true,
            ..Attr::default()
        };
        let true_color = style(attr, look(Depth::TrueColor));
        assert_eq!(true_color.fg, Some(Color::Rgb(250, 10, 10)));
        assert_eq!(true_color.bg, Some(Color::Indexed(21)));
        let sixteen = style(attr, look(Depth::Ansi16));
        assert_eq!(sixteen.fg, Some(Color::Indexed(9)));
        assert_eq!(sixteen.bg, Some(Color::Indexed(4)));
        let none = style(attr, look(Depth::None));
        assert_eq!((none.fg, none.bg), (None, None));
        for style in [true_color, sixteen, none] {
            assert!(
                style
                    .add_modifier
                    .contains(Modifier::BOLD | Modifier::REVERSED)
            );
        }
    }

    #[test]
    fn an_answer_is_a_screen_only_for_the_asked_identity_and_a_known_format() {
        let id = "11111111-1111-4111-8111-111111111111";
        let answer = |id: &str, format: &str| serde_json::json!({"identityId": id, "host": "tmux", "format": format, "text": "\u{1b}[31mhi\u{1b}[0m\nthere"});
        let Observation::Screen(screen) = from_value(id, &answer(id, "ansi"), 7) else {
            panic!("a screen");
        };
        assert_eq!((screen.rows.len(), screen.width, screen.read_ms), (2, 5, 7));
        for bad in [
            answer("22222222-2222-4222-8222-222222222222", "ansi"),
            answer(id, "html"),
            serde_json::json!({"identityId": id, "format": "ansi"}),
        ] {
            assert!(matches!(
                from_value(id, &bad, 0),
                Observation::Unavailable(_)
            ));
        }
    }

    #[test]
    fn width_counts_cells_not_characters() {
        let rows = parse("界a\u{200b}b");
        assert_eq!(width(&rows[0]), 4);
    }
}
