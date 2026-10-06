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

#[test]
fn flat_frame_keeps_title_styles_inner_content_and_square_defaults() {
    let area = Rect::new(0, 0, 10, 4);
    let mut buffer = Buffer::empty(area);
    buffer.set_string(0, 1, "Xkeep    X", Style::new().fg(Color::Green));
    let outline = outline(Line::from(vec![
        Span::raw(" ab "),
        Span::styled("c ", Style::new().fg(Color::Red)),
    ]));
    outline.paint_flat(area, &mut buffer);
    assert_eq!(
        rows(&buffer),
        ["─ ab c ───", " keep     ", "          ", "──────────"]
    );
    assert_eq!(
        buffer[(0, 1)].fg,
        Color::Blue,
        "blank former wall is styled"
    );
    assert_eq!(buffer[(1, 1)].fg, Color::Green, "content is not filled");
    assert_eq!(outline.inner(area), Rect::new(1, 1, 8, 2));
    assert!(buffer[(2, 0)].modifier.contains(Modifier::BOLD));
    assert_eq!(buffer[(5, 0)].fg, Color::Red);
    outline.paint(area, &mut buffer);
    assert_eq!(rows(&buffer)[0], "┌ ab c ──┐", "default remains square");
}

#[test]
fn flat_frame_changes_only_edge_symbols_when_clipped_or_tiny() {
    for buffer_area in [Rect::new(0, 0, 12, 6), Rect::new(3, 2, 5, 3)] {
        for area in [
            Rect::new(0, 0, 12, 6),
            Rect::new(2, 1, 10, 5),
            Rect::new(4, 3, 1, 1),
            Rect::new(4, 3, 2, 2),
            Rect::new(20, 20, 3, 3),
            Rect::new(4, 3, 0, 0),
        ] {
            let mut square = Buffer::empty(buffer_area);
            square.set_style(buffer_area, Style::new().bg(Color::Yellow));
            let mut flat = square.clone();
            let outline = outline(Line::raw("a\x1bb"));
            outline.paint(area, &mut square);
            outline.paint_flat(area, &mut flat);
            for (before, after) in square.content.iter().zip(&flat.content) {
                let mut expected = before.clone();
                match before.symbol() {
                    "┌" | "┐" | "└" | "┘" => {
                        expected.set_symbol("─");
                    }
                    "│" => {
                        expected.set_symbol(" ");
                    }
                    _ => {}
                }
                assert_eq!(&expected, after, "{buffer_area:?}/{area:?}");
            }
        }
    }
}
