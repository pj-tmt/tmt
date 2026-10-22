//! Another program's terminal text, read as data. Only colours and text attributes survive
//! parsing; every other escape sequence and control is dropped, so the text can never move
//! the cursor, retitle the terminal or write to the clipboard. The colours are the
//! program's own, not TMT's design tokens; with `ratatui` they are mapped to the terminal's
//! [`Depth`](crate::theme::Depth).
#[cfg(feature = "ratatui")]
use crate::theme::Depth;
#[cfg(feature = "ratatui")]
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthChar;

/// A colour as the screen named it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shade {
    /// 0–15 are the terminal's own colours; 16–255 the xterm cube and grays.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attr {
    pub fg: Option<Shade>,
    pub bg: Option<Shade>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

/// Text sharing one set of attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub attr: Attr,
}

pub type Row = Vec<Run>;

const MAX_PARAMS: usize = 32;

/// Rows of `text`. `ansi` screens carry SGR; `plain` ones carry none, and any escape left in
/// them is dropped just the same.
pub fn parse(text: &str) -> Vec<Row> {
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
            30..=37 => attr.fg = Some(Shade::Indexed((code - 30) as u8)),
            39 => attr.fg = None,
            40..=47 => attr.bg = Some(Shade::Indexed((code - 40) as u8)),
            49 => attr.bg = None,
            90..=97 => attr.fg = Some(Shade::Indexed((code - 90 + 8) as u8)),
            100..=107 => attr.bg = Some(Shade::Indexed((code - 100 + 8) as u8)),
            38 | 48 => {
                // `38;5;n` and `38;2;r;g;b`, or the colon forms `38:5:n`, `38:2:r:g:b` and
                // `38:2:cs:r:g:b`, read by position: the colour-space part may be empty.
                let parts: Vec<Option<u16>> = if sub.len() > 1 {
                    sub[1..].iter().map(|part| number(part)).collect()
                } else {
                    let take = match groups.get(index).and_then(|g| number(g)) {
                        Some(5) => 2,
                        Some(2) => 4,
                        _ => 0,
                    };
                    let parts = groups[index..(index + take).min(groups.len())]
                        .iter()
                        .map(|part| number(part))
                        .collect();
                    index += take;
                    parts
                };
                let byte = |at: usize| {
                    parts
                        .get(at)
                        .copied()
                        .flatten()
                        .and_then(|n| u8::try_from(n).ok())
                };
                let paint = match parts.first().copied().flatten() {
                    Some(5) => byte(1).map(Shade::Indexed),
                    Some(2) => {
                        // With a colour-space part there are five parts, without it four.
                        let from = if parts.len() >= 5 { 2 } else { 1 };
                        match (byte(from), byte(from + 1), byte(from + 2)) {
                            (Some(r), Some(g), Some(b)) => Some(Shade::Rgb(r, g, b)),
                            _ => None,
                        }
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
#[cfg(feature = "ratatui")]
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

#[cfg(feature = "ratatui")]
fn rgb(paint: Shade) -> (u8, u8, u8) {
    match paint {
        Shade::Rgb(r, g, b) => (r, g, b),
        Shade::Indexed(n) if n < 16 => BASE16[usize::from(n)],
        Shade::Indexed(n) if n < 232 => {
            let n = n - 16;
            let level = |step: u8| if step == 0 { 0 } else { 55 + 40 * step };
            (level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        Shade::Indexed(n) => {
            let gray = 8 + 10 * (n - 232);
            (gray, gray, gray)
        }
    }
}

#[cfg(feature = "ratatui")]
fn nearest16(paint: Shade) -> u8 {
    if let Shade::Indexed(n) = paint
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

#[cfg(feature = "ratatui")]
fn color(paint: Shade, depth: Depth) -> Option<Color> {
    match depth {
        Depth::None => None,
        Depth::Ansi16 => Some(Color::Indexed(nearest16(paint))),
        _ => Some(match paint {
            Shade::Indexed(n) => Color::Indexed(n),
            Shade::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }),
    }
}

#[cfg(feature = "ratatui")]
/// The style of `attr` on a terminal of `depth`: colours as far as it allows, attributes
/// always.
pub fn style(attr: Attr, depth: Depth) -> Style {
    let mut style = Style::new();
    if let Some(paint) = attr.fg
        && let Some(fg) = color(paint, depth)
    {
        style = style.fg(fg);
    }
    if let Some(paint) = attr.bg
        && let Some(bg) = color(paint, depth)
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
pub fn width(row: &Row) -> usize {
    row.iter()
        .flat_map(|run| run.text.chars())
        .map(|c| c.width().unwrap_or(0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "ratatui")]
    use ratatui::style::{Color, Modifier};

    fn flat(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| row.iter().map(|run| run.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn colours_and_attributes_split_a_row_into_runs() {
        let rows = parse("\u{1b}[31mred\u{1b}[39m plain \u{1b}[1;4mbold\u{1b}[0m");
        assert_eq!(flat(&rows), ["red plain bold"]);
        let attrs: Vec<_> = rows[0].iter().map(|run| run.attr).collect();
        assert_eq!(attrs[0].fg, Some(Shade::Indexed(1)));
        assert_eq!(attrs[1], Attr::default());
        assert!(attrs[2].bold && attrs[2].underline);
    }

    #[test]
    fn extended_colours_in_both_forms() {
        let rows =
            parse("\u{1b}[38;5;208ma\u{1b}[48;2;1;2;3mb\u{1b}[38:2::9:8:7mc\u{1b}[48:5:21md");
        let attrs: Vec<_> = rows[0].iter().map(|run| run.attr).collect();
        assert_eq!(attrs[0].fg, Some(Shade::Indexed(208)));
        assert_eq!(attrs[1].bg, Some(Shade::Rgb(1, 2, 3)));
        assert_eq!(attrs[2].fg, Some(Shade::Rgb(9, 8, 7)));
        assert_eq!(attrs[3].bg, Some(Shade::Indexed(21)));
        // Truncated or out-of-range colours leave the attribute alone.
        let rows = parse("\u{1b}[38;5;999mx\u{1b}[38;2;1mY\u{1b}[38;5m");
        assert_eq!(flat(&rows), ["xY"]);
        assert!(rows[0].iter().all(|run| run.attr == Attr::default()));
    }

    #[test]
    fn the_colon_form_is_read_by_position_with_or_without_a_colour_space() {
        let rows = parse(
            "\u{1b}[38:2:0:10:20:30ma\u{1b}[38:2::11:21:31mb\u{1b}[38:2:12:22:32mc\u{1b}[48:2:7:1:2:3md",
        );
        let attrs: Vec<_> = rows[0].iter().map(|run| run.attr).collect();
        assert_eq!(attrs[0].fg, Some(Shade::Rgb(10, 20, 30)));
        assert_eq!(attrs[1].fg, Some(Shade::Rgb(11, 21, 31)));
        assert_eq!(attrs[2].fg, Some(Shade::Rgb(12, 22, 32)));
        assert_eq!(attrs[3].bg, Some(Shade::Rgb(1, 2, 3)));
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
            ("j", Some(Shade::Indexed(1)))
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

    #[cfg(feature = "ratatui")]
    #[test]
    fn colours_follow_the_terminal_depth_and_attributes_always_stay() {
        let attr = Attr {
            fg: Some(Shade::Rgb(250, 10, 10)),
            bg: Some(Shade::Indexed(21)),
            bold: true,
            reverse: true,
            ..Attr::default()
        };
        let true_color = style(attr, Depth::TrueColor);
        assert_eq!(true_color.fg, Some(Color::Rgb(250, 10, 10)));
        assert_eq!(true_color.bg, Some(Color::Indexed(21)));
        let sixteen = style(attr, Depth::Ansi16);
        assert_eq!(sixteen.fg, Some(Color::Indexed(9)));
        assert_eq!(sixteen.bg, Some(Color::Indexed(4)));
        let none = style(attr, Depth::None);
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
    fn width_counts_cells_not_characters() {
        let rows = parse("界a\u{200b}b");
        assert_eq!(width(&rows[0]), 4);
    }
}
