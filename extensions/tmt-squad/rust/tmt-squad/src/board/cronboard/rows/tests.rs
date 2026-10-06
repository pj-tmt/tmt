use super::*;
use crate::board::cronboard::line::tests::{NOW, view};
use ratatui::{Terminal, backend::TestBackend};

fn paint(rows: Vec<Value>, columns: Columns, width: u16, height: u16) -> Vec<String> {
    let mut pane = super::super::surface::Pane::default();
    let mut screen = Terminal::new(TestBackend::new(width, height)).unwrap();
    screen
        .draw(|frame| {
            pane.render(
                rows.clone(),
                columns,
                "(no jobs)",
                frame,
                crate::look::Look::default(),
                frame.area(),
            )
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

#[test]
fn rows_show_state_owner_schedule_and_next_and_expand_in_place() {
    let a = view("tmt-lead", "merge queue sweep", Some(NOW + 3_600_000));
    let mut b = view("tmt-ops", "worktree cleanup\nsecond line", None);
    b.job.room_id = "room-b".into();
    b.job.pause = Some(tmt_squad::cron::Pause {
        by: "u".into(),
        at_ms: 0,
    });
    let id = row_id(&key_of(&a));
    let rows = project(&[&a, &b], Some(&id), None, NOW);
    let columns = Columns::for_width(true, 120);
    let screen = paint(rows, columns, 120, 8);
    assert!(screen[0].contains("● c0"), "{screen:#?}");
    assert!(screen[0].contains("every 30m"), "{screen:#?}");
    assert!(
        screen[0].contains("Mon 10-05 00:30") && screen[0].contains("tmt-core"),
        "{screen:#?}"
    );
    assert!(screen[1].starts_with("message"), "{screen:#?}");
    assert!(
        screen[2].starts_with("next     Mon 10-05 00:00 · Mon 10-05 00:30 · "),
        "{screen:#?}"
    );
    assert!(
        screen[3].contains("○") && screen[3].contains("paused"),
        "{screen:#?}"
    );
}

#[test]
fn the_message_preview_takes_what_the_fixed_tracks_leave_and_steps_aside() {
    let a = view("tmt-lead", "merge queue sweep", Some(NOW + 3_600_000));
    let rows = project(&[&a], None, None, NOW);
    for (width, squad, preview) in [
        (160, true, true),
        (93, true, true),
        (92, true, false),
        (80, false, true),
        (79, false, false),
        (74, false, false),
    ] {
        let screen = paint(rows.clone(), Columns::for_width(squad, width), width, 3);
        assert_eq!(
            screen[0].contains("merge queue sweep"),
            preview,
            "{width}: {screen:#?}"
        );
        assert!(screen[0].contains("every 30m"), "{width}: {screen:#?}");
        assert!(screen[0].contains("00:30"), "{width}: {screen:#?}");
    }
}

#[test]
fn row_ids_are_namespaced_and_untrusted_names_are_neutralized() {
    let mut a = view("o\u{1b}[2Jwner", "m\u{202e}essage", None);
    a.job.room_id = "6f1c2d3e-0000-4000-8000-000000000001".into();
    let rows = project(&[&a], None, None, NOW);
    assert_eq!(rows[0]["id"], "6f1c2d3e-0000-4000-8000-000000000001/c0");
    let shown = paint(rows, Columns::for_width(false, 120), 120, 2).join("\n");
    assert!(
        !shown.contains('\u{1b}') && !shown.contains('\u{202e}'),
        "{shown}"
    );
}
