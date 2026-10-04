use super::*;
use crate::board::cronboard::line::tests::{NOW, cron, view, views};
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
        reads: 2,
    }
}

fn paint(list: &List, state: &State, width: u16, height: u16) -> Vec<String> {
    let mut screen = Terminal::new(TestBackend::new(width, height)).unwrap();
    screen
        .draw(|frame| {
            list.render(state, NOW, None, frame, Look::default(), frame.area());
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
        // Two jobs: border 2 + rows 2 + position, status and footer = 7 lines,
        // docked at the bottom with the body above left to the base.
        let top = screen
            .iter()
            .position(|line| line.trim_start().starts_with('┌'))
            .unwrap_or_default();
        assert!(
            screen[top].contains("cron · all squads") && screen.len() - top == 7,
            "{text}"
        );
        assert!(screen[..top].iter().all(String::is_empty), "{text}");
        assert!(text.contains("alpha") && text.contains("beta"), "{text}");
        assert!(text.contains("2 jobs · no clock"), "{text}");
        assert!(
            text.contains("↑↓ choose · ⏎ open squad") && text.contains("Esc close"),
            "{text}"
        );
        assert_eq!(
            text.contains("merge queue sweep"),
            // The docked sheet is nine tenths wide from 100 columns; the preview
            // needs what the fixed tracks leave.
            width >= 110,
            "{width}\n{text}"
        );
    }
}

#[test]
fn the_box_grows_with_the_jobs_and_never_passes_the_body() {
    let few = jobs();
    let list = List::open(&few, None);
    let height = |state: &State, rows: u16| {
        paint(&list, state, 100, rows)
            .iter()
            .filter(|line| !line.is_empty())
            .count()
    };
    assert_eq!(height(&few, 30), 7);
    let mut many = jobs();
    let cron = many.cron.as_mut().unwrap();
    for index in 0..8 {
        let mut job = view("tmt-ops", "more", None);
        job.job.room_id = format!("room-{index}");
        cron.jobs.push(job);
    }
    assert_eq!(height(&many, 30), 15);
    // The modal never takes more than four fifths of a short body.
    assert!(height(&many, 10) <= 8, "{}", height(&many, 10));
}

#[test]
fn selection_follows_job_identity_across_refresh_and_reorder() {
    let state = jobs();
    let list = List::open(&state, None);
    paint(&list, &state, 120, 12);
    assert!(matches!(list.input(&key(KeyCode::Down)), Some(Input::None)));
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
    let Some(Input::Open(id)) = list.input(&key(KeyCode::Enter)) else {
        panic!("Enter opens the selected job");
    };
    assert!(id.starts_with("room-a"));
    assert!(matches!(list.input(&key(KeyCode::Esc)), Some(Input::Close)));
    // Geometry painted before a resize is discarded and cannot activate.
    list.invalidate();
    let click = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 20,
        row: 2,
        modifiers: KeyModifiers::NONE,
    });
    assert!(list.input(&click).is_none());
}

#[test]
fn an_empty_or_failed_read_says_so() {
    let empty = State {
        cron: Some(cron(vec![], ClockStatus::Unknown)),
        failure: Some("storage unreachable".into()),
        reads: 2,
    };
    let list = List::open(&empty, None);
    let text = paint(&list, &empty, 120, 8).join("\n");
    assert!(text.contains("(no jobs · n new)"), "{text}");
    assert!(
        text.contains("0 jobs · clock: unknown · ! storage unreachable"),
        "{text}"
    );
    assert!(matches!(
        list.input(&key(KeyCode::Enter)),
        Some(Input::None)
    ));
}

fn snapshots() -> serde_json::Value {
    let mut frames = Vec::new();
    for (scenario, state) in [
        ("two squads", jobs()),
        (
            "empty",
            State {
                cron: Some(cron(vec![], ClockStatus::Unknown)),
                failure: Some("storage unreachable".into()),
                reads: 2,
            },
        ),
    ] {
        for width in [160u16, 100, 80] {
            let list = List::open(&state, None);
            frames.push(serde_json::json!({
                "scenario": scenario, "width": width, "lines": paint(&list, &state, width, 9),
            }));
        }
    }
    serde_json::json!(frames)
}

#[test]
fn list_snapshots_at_each_width() {
    assert_eq!(
        snapshots(),
        serde_json::from_str::<serde_json::Value>(include_str!("snapshots.json")).unwrap()
    );
}

#[test]
#[ignore = "explicit initial captures of the new c list; frozen parity is separate"]
fn record_list_snapshots() {
    std::fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/board/cronboard/list/snapshots.json"
        ),
        serde_json::to_string_pretty(&snapshots()).unwrap() + "\n",
    )
    .unwrap();
}

/// One press moves exactly one job, in the order shown, down and back up, and the
/// selected job stays painted (the list scrolls when the body is short).
#[test]
fn up_and_down_move_one_job_per_press_and_keep_the_selection_visible() {
    let state = State {
        cron: Some(cron(views(8, "alpha", "room-a"), ClockStatus::NoClock)),
        failure: None,
        reads: 2,
    };
    let order: Vec<String> = state
        .cron
        .iter()
        .flat_map(|cron| &cron.jobs)
        .map(|view| row_id(&key_of(view)))
        .collect();
    for height in [30, 9] {
        for (down, up) in [
            (KeyCode::Down, KeyCode::Up),
            (KeyCode::Char('j'), KeyCode::Char('k')),
        ] {
            let list = List::open(&state, None);
            paint(&list, &state, 100, height);
            assert_eq!(list.selected().as_deref(), Some(order[0].as_str()));
            let walk = |code: KeyCode, expected: &str| {
                assert!(matches!(list.input(&key(code)), Some(Input::None)));
                assert_eq!(
                    list.selected().as_deref(),
                    Some(expected),
                    "height {height}"
                );
                let screen = paint(&list, &state, 100, height);
                let cid = expected.rsplit('/').next().unwrap();
                assert!(
                    screen.iter().any(|line| line.contains(&format!(" {cid} "))),
                    "{cid} stays painted at height {height}\n{}",
                    screen.join("\n")
                );
            };
            for expected in &order[1..] {
                walk(down, expected);
            }
            for expected in order[..order.len() - 1].iter().rev() {
                walk(up, expected);
            }
        }
    }
}
