use super::*;
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}
fn paint(line: Line<'_>, width: u16, right: bool) -> Buffer {
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, 2));
    let area = Rect::new(0, 1, width, 1);
    let theme = Theme::default();
    if right {
        paint_right(&mut buffer, area, line, &theme, Depth::TrueColor);
    } else {
        paint_left(&mut buffer, area, line, &theme, Depth::TrueColor);
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
    let theme = Theme::default();
    for area in [
        Rect::new(0, 0, 0, 1),
        Rect::new(10, 5, 8, 1),
        Rect::new(2, 0, 8, 1),
    ] {
        paint_left(
            &mut buffer,
            area,
            Line::raw("xyz"),
            &theme,
            Depth::TrueColor,
        );
        paint_right(
            &mut buffer,
            area,
            Line::raw("xyz"),
            &theme,
            Depth::TrueColor,
        );
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
