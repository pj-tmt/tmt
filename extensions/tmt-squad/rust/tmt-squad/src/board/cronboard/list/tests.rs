use super::*;
use crate::board::cronboard::line::tests::{NOW, cron, view};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind},
};
use tmt_squad::cron::ClockStatus;

fn jobs() -> State {
    let mut a = view("tmt-lead", "merge queue sweep", Some(NOW + 3_600_000));
    a.job.room_id = "room-a".into();
    a.job.squad = "alpha".into();
    let mut b = view("tmt-ops", "worktree cleanup", None);
    b.job.room_id = "room-b".into();
    b.job.squad = "beta".into();
    State {
        cron: Some(cron(vec![a, b], ClockStatus::NoClock)),
        failure: None,
    }
}

fn paint(list: &List, state: &State, width: u16, height: u16) -> Vec<String> {
    let mut screen = Terminal::new(TestBackend::new(width, height)).unwrap();
    screen
        .draw(|frame| {
            list.render(state, NOW, frame, Look::default(), frame.area());
        })
        .unwrap();
    let buffer = screen.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn the_list_shows_every_squad_clock_and_keys_inside_its_box_at_each_width() {
    let state = jobs();
    let list = List::open(&state, None);
    for width in [160, 100, 80] {
        let screen = paint(&list, &state, width, 12);
        let text = screen.join("\n");
        assert!(
            screen[0].starts_with('┌') && screen[0].contains("cron · all squads"),
            "{text}"
        );
        assert!(text.contains("alpha") && text.contains("beta"), "{text}");
        assert!(text.contains("2 jobs · no clock"), "{text}");
        assert!(
            text.contains("↑↓ choose · ⏎ open squad · Esc close"),
            "{text}"
        );
        assert_eq!(
            text.contains("merge queue sweep"),
            width >= 100,
            "{width}\n{text}"
        );
    }
}

#[test]
fn selection_follows_job_identity_across_refresh_and_reorder() {
    let state = jobs();
    let list = List::open(&state, None);
    paint(&list, &state, 120, 12);
    assert!(matches!(list.input(&key(KeyCode::Down)), Input::None));
    let second = list.selected().unwrap();
    assert!(
        second.ends_with("/c0") && second.starts_with("room-b"),
        "{second}"
    );
    let mut reordered = jobs();
    reordered.cron.as_mut().unwrap().jobs.reverse();
    paint(&list, &reordered, 120, 12);
    assert_eq!(list.selected().as_deref(), Some(second.as_str()));
    // The selected job disappears: the nearest survivor takes over.
    reordered.cron.as_mut().unwrap().jobs.remove(0);
    paint(&list, &reordered, 120, 12);
    assert!(list.selected().unwrap().starts_with("room-a"));
}

#[test]
fn enter_opens_the_selected_job_and_a_stale_frame_never_activates_a_click() {
    let state = jobs();
    let list = List::open(&state, None);
    paint(&list, &state, 120, 12);
    let Input::Open(id) = list.input(&key(KeyCode::Enter)) else {
        panic!("Enter opens the selected job");
    };
    assert!(id.starts_with("room-a"));
    assert!(matches!(list.input(&key(KeyCode::Esc)), Input::Close));
    // Geometry painted before a resize is discarded and cannot activate.
    list.invalidate();
    let click = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 20,
        row: 2,
        modifiers: KeyModifiers::NONE,
    });
    assert!(matches!(list.input(&click), Input::None));
}

#[test]
fn an_empty_or_failed_read_says_so() {
    let empty = State {
        cron: Some(cron(vec![], ClockStatus::Unknown)),
        failure: Some("storage unreachable".into()),
    };
    let list = List::open(&empty, None);
    let text = paint(&list, &empty, 120, 8).join("\n");
    assert!(text.contains("(no jobs · n new)"), "{text}");
    assert!(
        text.contains("0 jobs · clock: unknown · ! storage unreachable"),
        "{text}"
    );
    assert!(matches!(list.input(&key(KeyCode::Enter)), Input::None));
}
