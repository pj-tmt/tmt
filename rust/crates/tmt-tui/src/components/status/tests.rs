use super::*;
use ratatui::style::{Color, Modifier, Style};

fn line(buffer: &Buffer, area: Rect) -> String {
    (area.left()..area.right())
        .map(|x| buffer[(x, area.y)].symbol())
        .collect()
}

fn draw(label: StatusLabel<'_>, frame: Option<usize>, depth: Depth, width: u16) -> Buffer {
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, 1));
    let area = buffer.area;
    StatusSlot { label, frame }.paint(&mut buffer, area, &Theme::default(), depth);
    buffer
}

#[test]
fn ages_floor_at_unit_boundaries_and_keep_all_english_wording_in_one_formatter() {
    for (seconds, expected) in [
        (0, "cached 0s ago"),
        (59, "cached 59s ago"),
        (60, "cached 1m ago"),
        (119, "cached 1m ago"),
        (120, "cached 2m ago"),
        (3599, "cached 59m ago"),
        (3600, "cached 1h ago"),
        (86399, "cached 23h ago"),
        (86400, "cached 1d ago"),
        (172799, "cached 1d ago"),
        (172800, "cached 2d ago"),
        (u64::MAX, "cached 213503982334601d ago"),
    ] {
        assert_eq!(age_label("cached", Duration::from_secs(seconds)), expected);
    }
    assert_eq!(age_label("", Duration::from_millis(999)), "0s ago");
    assert_eq!(
        age_label("saved", Duration::from_millis(119999)),
        "saved 1m ago"
    );
}

#[test]
fn status_text_and_style_snapshots_include_the_steady_no_color_form() {
    for (depth, busy, age, idle, fg, modifier) in [
        (
            Depth::TrueColor,
            "⠋ updating          ",
            "⠋ cached 2m ago     ",
            "cached 2m ago       ",
            Color::Rgb(133, 133, 133),
            Modifier::empty(),
        ),
        (
            Depth::Ansi16,
            "⠋ updating          ",
            "⠋ cached 2m ago     ",
            "cached 2m ago       ",
            Color::Reset,
            Modifier::DIM,
        ),
        (
            Depth::None,
            "[busy] updating     ",
            "[busy] cached 2m ago",
            "cached 2m ago       ",
            Color::Reset,
            Modifier::empty(),
        ),
    ] {
        for (label, frame, expected) in [
            (StatusLabel::Text("updating"), Some(0), busy),
            (
                StatusLabel::Age {
                    prefix: "cached",
                    age: Duration::from_secs(120),
                },
                Some(0),
                age,
            ),
            (
                StatusLabel::Age {
                    prefix: "cached",
                    age: Duration::from_secs(120),
                },
                None,
                idle,
            ),
        ] {
            let buffer = draw(label, frame, depth, 20);
            assert_eq!(line(&buffer, buffer.area), expected, "{depth:?}");
            for cell in &buffer.content {
                assert_eq!(cell.fg, fg, "{depth:?}");
                assert_eq!(cell.bg, Color::Reset, "{depth:?}");
                assert_eq!(cell.modifier, modifier, "{depth:?}");
            }
        }
    }
}

#[test]
fn frames_wrap_and_ticks_change_only_the_marker_while_no_color_never_animates() {
    let expected = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let label = StatusLabel::Text("updating");
    let first = draw(label, Some(0), Depth::TrueColor, 20);
    let steady = draw(label, Some(0), Depth::None, 20);
    for frame in (0..20).chain([usize::MAX]) {
        let buffer = draw(label, Some(frame), Depth::TrueColor, 20);
        assert_eq!(buffer[(0, 0)].symbol(), expected[frame % 10]);
        assert_eq!(&buffer.content[1..], &first.content[1..]);
        assert_eq!(draw(label, Some(frame), Depth::None, 20), steady);
    }
}

#[test]
fn shorter_idle_and_empty_labels_replace_the_whole_owned_row_only() {
    let bounds = Rect::new(2, 3, 24, 4);
    let mut buffer = Buffer::empty(bounds);
    for cell in &mut buffer.content {
        cell.set_symbol("X").set_style(
            Style::new()
                .fg(Color::Red)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );
    }
    let area = Rect::new(4, 4, 20, 3);
    let before = buffer.clone();
    for (label, frame, expected) in [
        ("a longer label", Some(1), "⠙ a longer label    "),
        ("ok", None, "ok                  "),
        ("", Some(0), "⠋                   "),
        ("", None, "                    "),
    ] {
        StatusSlot {
            label: StatusLabel::Text(label),
            frame,
        }
        .paint(&mut buffer, area, &Theme::default(), Depth::TrueColor);
        assert_eq!(line(&buffer, area), expected);
        for y in bounds.top()..bounds.bottom() {
            for x in bounds.left()..bounds.right() {
                if !Rect::new(area.x, area.y, area.width, 1).contains((x, y).into()) {
                    assert_eq!(
                        buffer[(x, y)],
                        before[(x, y)],
                        "outside the owned first row"
                    );
                }
            }
        }
        assert_eq!(
            buffer[(area.right() - 1, area.y)].fg,
            Color::Rgb(133, 133, 133)
        );
    }
}

#[test]
fn fitting_escapes_controls_and_never_splits_graphemes_or_a_busy_token() {
    let label = StatusLabel::Text("updating");
    let buffer = draw(label, Some(0), Depth::None, 5);
    assert_eq!(line(&buffer, buffer.area), "updat");
    let buffer = draw(StatusLabel::Text(""), Some(0), Depth::None, 6);
    assert_eq!(line(&buffer, buffer.area), "[busy]");
    let buffer = draw(StatusLabel::Text("a\x1b[31m\n\t"), None, Depth::None, 24);
    assert!(!line(&buffer, buffer.area).chars().any(char::is_control));
    assert_eq!(line(&buffer, buffer.area), "a\\u{1b}[31m\\n\\t         ");
    let buffer = draw(StatusLabel::Text("ab世"), None, Depth::None, 3);
    assert_eq!(line(&buffer, buffer.area), "ab ");
    let buffer = draw(StatusLabel::Text("e\u{301}x"), None, Depth::None, 1);
    assert_eq!(buffer[(0, 0)].symbol(), "e\u{301}");
}

#[test]
fn empty_and_clipped_rectangles_respect_nonzero_buffer_origins() {
    let mut buffer = Buffer::empty(Rect::new(3, 2, 8, 3));
    let before = buffer.clone();
    let slot = StatusSlot {
        label: StatusLabel::Text("abcdefgh"),
        frame: None,
    };
    for area in [
        Rect::new(3, 2, 0, 1),
        Rect::new(3, 2, 8, 0),
        Rect::new(20, 20, 4, 1),
        Rect::new(3, 1, 8, 3),
    ] {
        slot.paint(&mut buffer, area, &Theme::default(), Depth::None);
        assert_eq!(buffer, before);
    }
    slot.paint(
        &mut buffer,
        Rect::new(1, 2, 8, 2),
        &Theme::default(),
        Depth::None,
    );
    assert_eq!(line(&buffer, Rect::new(3, 2, 8, 1)), "cdefgh  ");
    slot.paint(
        &mut buffer,
        Rect::new(9, 3, 8, 1),
        &Theme::default(),
        Depth::None,
    );
    assert_eq!(line(&buffer, Rect::new(3, 3, 8, 1)), "      ab");
    assert_eq!(line(&buffer, Rect::new(3, 4, 8, 1)), "        ");
    let busy = StatusSlot {
        label: StatusLabel::Text("updating"),
        frame: Some(0),
    };
    // Logical room for a marker is insufficient when the buffer clips it.
    busy.paint(
        &mut buffer,
        Rect::new(9, 3, 8, 1),
        &Theme::default(),
        Depth::None,
    );
    assert_eq!(line(&buffer, Rect::new(3, 3, 8, 1)), "      up");
    busy.paint(
        &mut buffer,
        Rect::new(1, 4, 8, 1),
        &Theme::default(),
        Depth::None,
    );
    assert_eq!(line(&buffer, Rect::new(3, 4, 8, 1)), "dating  ");
}
