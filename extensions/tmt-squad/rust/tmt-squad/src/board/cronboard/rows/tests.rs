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
    let rows = project(
        &[&a, &b],
        &[crate::board::row_detail::Target::Job(id)],
        None,
        NOW,
        80,
        crate::look::Look::default(),
        Columns::for_width(true, 80),
    );
    let columns = Columns::for_width(true, 80);
    let screen = paint(rows, columns, 80, 8);
    assert!(screen[0].contains("● c0"), "{screen:#?}");
    assert!(screen[0].contains("every 30m"), "{screen:#?}");
    assert!(
        screen[0].contains("Mon 10-05 00:30") && screen[0].contains("tmt-core"),
        "{screen:#?}"
    );
    assert!(
        screen[1].contains("│prompt") && screen[1].contains("merge queue sweep"),
        "{screen:#?}"
    );
    assert!(
        !screen.iter().any(|line| line.contains("│when")
            || line.contains("│next")
            || line.contains("│target")),
        "already visible fields are not repeated: {screen:#?}"
    );
    assert!(
        screen
            .iter()
            .any(|line| line.contains("○") && line.contains("paused")),
        "{screen:#?}"
    );
}

#[test]
fn the_message_preview_takes_what_the_fixed_tracks_leave_and_steps_aside() {
    let a = view("tmt-lead", "merge queue sweep", Some(NOW + 3_600_000));
    let rows = project(
        &[&a],
        &[],
        None,
        NOW,
        120,
        crate::look::Look::default(),
        Columns::for_width(false, 120),
    );
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
    let rows = project(
        &[&a],
        &[],
        None,
        NOW,
        120,
        crate::look::Look::default(),
        Columns::for_width(false, 120),
    );
    assert_eq!(rows[0]["id"], "6f1c2d3e-0000-4000-8000-000000000001/c0");
    let shown = paint(rows, Columns::for_width(false, 120), 120, 2).join("\n");
    assert!(
        !shown.contains('\u{1b}') && !shown.contains('\u{202e}'),
        "{shown}"
    );
}

#[test]
fn clipped_prompt_and_target_remain_in_detail_until_the_whole_field_is_visible() {
    let message = "Review the long release checklist before shipping";
    let mut job = view("tmt-lead", message, None);
    job.job.pause = Some(tmt_squad::cron::Pause {
        by: "u".into(),
        at_ms: 0,
    });
    for (width, hidden) in [(80, true), (160, false)] {
        let columns = Columns::for_width(false, width);
        let data = expanded_detail(&job, NOW, columns, width);
        assert_eq!(
            data["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field["label"] == "prompt"),
            hidden
        );
        let rows = project(
            &[&job],
            &[crate::board::row_detail::Target::Job(detail_id(&job))],
            None,
            NOW,
            width,
            crate::look::Look::default(),
            columns,
        );
        let screen = paint(rows, columns, width, 8);
        if hidden {
            assert!(!screen[0].contains(message), "{screen:#?}");
            assert!(
                screen.iter().any(|line| line.contains(message)),
                "{screen:#?}"
            );
            assert!(!screen.iter().any(|line| line.contains("no details yet")));
        } else {
            assert!(screen[0].contains(message));
            assert!(
                screen[1].starts_with("      │no details yet"),
                "{screen:#?}"
            );
        }
    }
    job.owner_name = Some("a target name longer than sixteen cells".into());
    for width in [80, 160] {
        let data = expanded_detail(&job, NOW, Columns::for_width(false, width), width);
        assert!(
            data["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field["label"] == "target"
                    && field["text"] == job.owner_name.as_deref().unwrap())
        );
    }
    job.job.message = "first line\nsecond line".into();
    let data = expanded_detail(&job, NOW, Columns::for_width(false, 160), 160);
    assert!(
        data["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["label"] == "prompt" && field["text"] == "first line\nsecond line")
    );
}
