use super::*;
use ratatui::{Terminal, backend::TestBackend};

fn lines(count: usize) -> Vec<Line<'static>> {
    (0..count)
        .map(|index| Line::from(format!("line {index}")))
        .collect()
}

/// Draws `count` lines for `pane` in a 20x`height` area and returns the
/// screen text.
fn draw(scrolls: &Scrolls, pane: Pane, count: usize, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(20, height)).unwrap();
    terminal
        .draw(|frame| {
            scrolls.begin_frame();
            scrolls.show(frame, pane, frame.area(), lines(count), Style::new());
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..20)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

#[test]
fn content_that_fits_shows_whole_with_no_indicator() {
    let scrolls = Scrolls::default();
    assert_eq!(
        draw(&scrolls, Pane::Notes, 3, 4),
        ["line 0", "line 1", "line 2", ""]
    );
    scrolls.scroll(Pane::Notes, Step::Lines(5));
    assert_eq!(scrolls.offset(Pane::Notes), 0, "nothing to scroll");
}

#[test]
fn overflow_reserves_a_line_for_what_is_above_and_below() {
    let scrolls = Scrolls::default();
    let screen = draw(&scrolls, Pane::Notes, 10, 4);
    assert_eq!(&screen[..3], ["line 0", "line 1", "line 2"]);
    assert_eq!(screen[3].trim(), "7 more ↓");
    scrolls.scroll(Pane::Notes, Step::Lines(3));
    let screen = draw(&scrolls, Pane::Notes, 10, 4);
    assert_eq!(&screen[..3], ["line 3", "line 4", "line 5"]);
    assert_eq!(screen[3].trim(), "↑ 3  4 more ↓");
    scrolls.scroll(Pane::Notes, Step::Bottom);
    let screen = draw(&scrolls, Pane::Notes, 10, 4);
    assert_eq!(&screen[..3], ["line 7", "line 8", "line 9"]);
    assert_eq!(screen[3].trim(), "↑ 7");
}

#[test]
fn steps_stay_within_the_content_and_pages_keep_a_line_of_context() {
    let scrolls = Scrolls::default();
    draw(&scrolls, Pane::Replies, 30, 11);
    // Ten lines shown at a time; a page moves nine.
    scrolls.scroll(Pane::Replies, Step::Pages(1));
    assert_eq!(scrolls.offset(Pane::Replies), 9);
    scrolls.scroll(Pane::Replies, Step::Pages(5));
    assert_eq!(scrolls.offset(Pane::Replies), 20, "clamped at the bottom");
    scrolls.scroll(Pane::Replies, Step::Lines(-100));
    assert_eq!(scrolls.offset(Pane::Replies), 0);
    // Content that shrinks pulls the position back on the next draw.
    scrolls.scroll(Pane::Replies, Step::Bottom);
    draw(&scrolls, Pane::Replies, 12, 11);
    assert_eq!(scrolls.offset(Pane::Replies), 2);
}

#[test]
fn each_pane_keeps_its_own_position_and_the_wheel_finds_the_pane_under_it() {
    let scrolls = Scrolls::default();
    let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
    terminal
        .draw(|frame| {
            scrolls.begin_frame();
            let left = Rect::new(0, 0, 20, 6);
            let right = Rect::new(20, 0, 20, 6);
            scrolls.show(frame, Pane::Rows, left, lines(20), Style::new());
            scrolls.show(frame, Pane::Notes, right, lines(20), Style::new());
        })
        .unwrap();
    assert_eq!(scrolls.pane_at(3, 2), Some(Pane::Rows));
    assert_eq!(scrolls.pane_at(25, 5), Some(Pane::Notes));
    assert_eq!(scrolls.pane_at(3, 6), None);
    scrolls.scroll(Pane::Notes, Step::Lines(WHEEL_LINES as isize));
    assert_eq!(
        (scrolls.offset(Pane::Rows), scrolls.offset(Pane::Notes)),
        (0, 3)
    );
}

#[test]
fn reveal_moves_only_as_far_as_needed() {
    let scrolls = Scrolls::default();
    let area = Rect::new(0, 0, 20, 5);
    // 20 lines in 5 rows: 4 shown.
    scrolls.reveal_range(Pane::Rows, 2..3, area, 20);
    assert_eq!(scrolls.offset(Pane::Rows), 0);
    scrolls.reveal_range(Pane::Rows, 9..10, area, 20);
    assert_eq!(scrolls.offset(Pane::Rows), 6);
    scrolls.reveal_range(Pane::Rows, 7..8, area, 20);
    assert_eq!(scrolls.offset(Pane::Rows), 6, "already visible");
    scrolls.reveal_range(Pane::Rows, 1..2, area, 20);
    assert_eq!(scrolls.offset(Pane::Rows), 1);
}

#[test]
fn indicator_uses_the_resolved_token_without_extra_dimming() {
    let look = crate::look::Look::default();
    let dim = look.role(tmt_cli_style::Role::Dim);
    let mut terminal = Terminal::new(TestBackend::new(20, 4)).unwrap();
    terminal
        .draw(|frame| {
            Scrolls::default().show(frame, Pane::Notes, frame.area(), lines(10), dim);
        })
        .unwrap();
    let marker = &terminal.backend().buffer()[(17, 3)];
    assert_eq!(marker.symbol(), "↓");
    assert_eq!(Some(marker.fg), dim.fg);
    assert!(!marker.modifier.contains(ratatui::style::Modifier::DIM));
}

#[test]
fn selection_reveals_all_visual_lines_or_the_start_of_a_tall_record() {
    let scrolls = Scrolls::default();
    let area = Rect::new(0, 0, 20, 6);
    scrolls.reveal_range(Pane::Rows, 4..7, area, 20);
    assert_eq!(scrolls.offset(Pane::Rows), 2);
    scrolls.reveal_range(Pane::Rows, 1..4, area, 20);
    assert_eq!(scrolls.offset(Pane::Rows), 1);
    scrolls.reveal_range(Pane::Rows, 10..18, area, 20);
    assert_eq!(scrolls.offset(Pane::Rows), 6);
}
