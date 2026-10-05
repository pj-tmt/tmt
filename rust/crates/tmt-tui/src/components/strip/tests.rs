use super::*;
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};
use tmt_cli_style::{Depth, Theme};

fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}
fn paint(line: Line<'_>, width: u16, right: bool) -> Buffer {
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, 2));
    let area = Rect::new(0, 1, width, 1);
    if right {
        paint_right(&mut buffer, area, line);
    } else {
        paint_left(&mut buffer, area, line);
    }
    buffer
}

#[test]
fn spans_keep_their_style_over_the_line_style() {
    let line = Line::from(vec![
        Span::styled("ab", Style::new().fg(Color::Red)),
        Span::raw("cd"),
    ])
    .style(Style::new().add_modifier(Modifier::BOLD));
    let buffer = paint(line, 6, false);
    assert_eq!(row(&buffer, 1), "abcd  ");
    assert_eq!(buffer[(0, 1)].fg, Color::Red);
    assert!(buffer[(0, 1)].modifier.contains(Modifier::BOLD));
    assert!(buffer[(2, 1)].modifier.contains(Modifier::BOLD));
    assert_ne!(buffer[(2, 1)].fg, Color::Red);
    assert_eq!(row(&buffer, 0), "      ", "other lines stay untouched");
}

#[test]
fn text_is_clipped_to_the_area_and_wide_graphemes_never_straddle_the_edge() {
    assert_eq!(row(&paint(Line::raw("abcdefgh"), 4, false), 1), "abcd");
    let buffer = paint(Line::raw("ab世界"), 3, false);
    assert_eq!(
        row(&buffer, 1),
        "ab ",
        "a wide grapheme at the edge is dropped"
    );
}

#[test]
fn right_alignment_ends_at_the_right_edge_and_long_lines_keep_their_start() {
    assert_eq!(row(&paint(Line::raw("abc"), 6, true), 1), "   abc");
    assert_eq!(row(&paint(Line::raw("abcdefgh"), 4, true), 1), "abcd");
}

#[test]
fn control_characters_are_escaped_not_sent_to_the_terminal() {
    let buffer = paint(Line::raw("a\x1b[31mb"), 12, false);
    assert!(!row(&buffer, 1).contains('\x1b'), "{:?}", row(&buffer, 1));
}

#[test]
fn an_area_outside_the_buffer_or_without_width_paints_nothing_and_never_panics() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
    for area in [
        Rect::new(0, 0, 0, 1),
        Rect::new(10, 5, 8, 1),
        Rect::new(2, 0, 8, 1),
    ] {
        paint_left(&mut buffer, area, Line::raw("xyz"));
        paint_right(&mut buffer, area, Line::raw("xyz"));
    }
    assert_eq!(row(&buffer, 0), "  xy");
}

#[test]
fn a_wide_grapheme_continuation_cell_carries_its_span_style() {
    let line = Line::from(vec![Span::styled("世", Style::new().fg(Color::Green))]);
    let buffer = paint(line, 4, false);
    assert_eq!(buffer[(0, 1)].symbol(), "世");
    assert_eq!(buffer[(0, 1)].fg, Color::Green);
    assert_eq!(buffer[(1, 1)].fg, Color::Green, "continuation cell");
    assert_ne!(
        buffer[(2, 1)].fg,
        Color::Green,
        "cells after the text are untouched"
    );
}

/// Small deterministic generator: the differential test must not depend on a crate or the clock.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % below
    }
}

fn random_style(random: &mut Lcg) -> Style {
    let colors = [
        Color::Red,
        Color::Rgb(1, 2, 3),
        Color::Indexed(9),
        Color::Reset,
    ];
    let mut style = Style::new();
    if random.next(2) == 0 {
        style = style.fg(colors[random.next(colors.len())]);
    }
    if random.next(3) == 0 {
        style = style.bg(colors[random.next(colors.len())]);
    }
    if random.next(3) == 0 {
        style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    }
    if random.next(5) == 0 {
        style = style.remove_modifier(Modifier::BOLD);
    }
    style
}

fn random_line(random: &mut Lcg) -> Line<'static> {
    const PIECES: [&str; 17] = [
        "a",
        "bc",
        "def ghi",
        "世界",
        "世",
        "e\u{301}",
        "\u{200b}",
        "\x1b[31m",
        "\t",
        "a\nb",
        "👨‍👩‍👧",
        "…",
        " ",
        "",
        "\u{7f}",
        "ｱ",
        "x\u{0}y",
    ];
    let spans = (0..random.next(7))
        .map(|_| {
            let text: String = (0..1 + random.next(4))
                .map(|_| PIECES[random.next(PIECES.len())])
                .collect();
            Span::styled(text, random_style(random))
        })
        .collect::<Vec<_>>();
    Line::from(spans).style(random_style(random))
}

/// A buffer that already holds styled text, so the fill and `reset` are observable.
fn occupied(bounds: Rect) -> Buffer {
    let mut buffer = Buffer::empty(bounds);
    for y in bounds.top()..bounds.bottom() {
        for x in bounds.left()..bounds.right() {
            let cell = &mut buffer[(x, y)];
            cell.set_symbol(if (x + y) % 2 == 0 { "#" } else { "世" });
            cell.set_style(Style::new().fg(Color::Blue).add_modifier(Modifier::ITALIC));
        }
    }
    buffer
}

#[test]
fn direct_painting_equals_the_layout_pipeline_cell_for_cell() {
    let theme = Theme::default();
    let mut random = Lcg(0x5eed);
    for case in 0..6000 {
        // The buffer need not start at the origin: an area can begin above or left of it.
        let bounds = Rect::new(
            random.next(3) as u16,
            random.next(3) as u16,
            1 + random.next(24) as u16,
            1 + random.next(3) as u16,
        );
        // Areas reach past every buffer edge and may have no width or height.
        let area = Rect::new(
            random.next(12) as u16,
            random.next(4) as u16,
            random.next(26) as u16,
            random.next(4) as u16,
        );
        let line = random_line(&mut random);
        for depth in [Depth::TrueColor, Depth::None] {
            let mut expected = occupied(bounds);
            let mut actual = occupied(bounds);
            if case % 2 == 0 {
                reference::paint_left(&mut expected, area, line.clone(), &theme, depth);
                paint_left(&mut actual, area, line.clone());
            } else {
                reference::paint_right(&mut expected, area, line.clone(), &theme, depth);
                paint_right(&mut actual, area, line.clone());
            }
            assert_eq!(
                actual, expected,
                "case {case}: {line:?} in {area:?} on {bounds:?}"
            );
        }
    }
}
