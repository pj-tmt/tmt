use super::*;
use ratatui::style::{Color, Modifier};

fn rows(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}
fn outline(title: Line<'static>) -> Outline<'static> {
    Outline {
        title,
        border: Style::new().fg(Color::Blue),
        title_style: Style::new().add_modifier(Modifier::BOLD),
    }
}

#[test]
fn draws_square_borders_and_a_title_without_clearing_the_inside() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 4));
    buffer.set_string(1, 1, "keep", Style::new());
    let outline = outline(Line::from(vec![
        Span::raw(" ab "),
        Span::styled("c ", Style::new().fg(Color::Red)),
    ]));
    outline.paint(Rect::new(0, 0, 10, 4), &mut buffer);
    assert_eq!(
        rows(&buffer),
        ["┌ ab c ──┐", "│keep    │", "│        │", "└────────┘"]
    );
    assert_eq!(buffer[(0, 0)].fg, Color::Blue);
    assert!(
        buffer[(2, 0)].modifier.contains(Modifier::BOLD),
        "title style"
    );
    assert_eq!(buffer[(5, 0)].fg, Color::Red, "span style wins");
    assert_eq!(outline.inner(Rect::new(0, 0, 10, 4)), Rect::new(1, 1, 8, 2));
}

#[test]
fn an_empty_title_draws_none_and_titles_are_escaped() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 3));
    outline(Line::default()).paint(Rect::new(0, 0, 8, 3), &mut buffer);
    assert_eq!(rows(&buffer)[0], "┌──────┐");
    let mut buffer = Buffer::empty(Rect::new(0, 0, 16, 3));
    outline(Line::raw("a\x1bb")).paint(Rect::new(0, 0, 16, 3), &mut buffer);
    assert!(!rows(&buffer)[0].contains('\x1b'), "{:?}", rows(&buffer)[0]);
}

#[test]
fn an_area_larger_than_the_buffer_is_clipped_not_a_panic() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
    outline(Line::raw("t")).paint(Rect::new(0, 0, 9, 9), &mut buffer);
    outline(Line::raw("t")).paint(Rect::new(20, 20, 3, 3), &mut buffer);
}
