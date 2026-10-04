//! The squad tab's jobs half: composition, focus transfer, scoped keys, row labels.
use super::*;
use crate::board::cronboard::{TEST_NOW, test_cron, test_view};
use std::time::Instant;
use tmt_squad::cron::ClockStatus;

fn members() -> Value {
    json!([{"rows": [
        row("alpha", "working", "task one", json!({"id": "u1"})),
        row("bravo", "idle", "task two", json!({"id": "u2"})),
        row("carol", "idle", "task three", json!({"id": "u3"})),
    ]}])
}

fn squad_tab() -> App {
    let mut app = board(members());
    let document = &mut app.view.as_mut().unwrap().document;
    document["squad"]["roomId"] = json!("room");
    let jobs = ["u1", "u3"].map(|owner| {
        let mut job = test_view(
            "someone",
            &format!("job for {owner}"),
            Some(TEST_NOW + 3_600_000),
        );
        job.job.squad = "product".into();
        job.job.room_id = "room".into();
        job.job.owner_id = Some(owner.into());
        job
    });
    let mut jobs = jobs.to_vec();
    // Distinct job ids come from the store; the fixture reuses one, so move the room.
    jobs[1].job.room_id = "room".into();
    app.cron
        .replace(Ok(test_cron(vec![jobs.remove(0)], ClockStatus::NoClock)));
    app
}

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn members_sit_above_the_squads_jobs_with_the_clock_on_the_rule() {
    let app = squad_tab();
    for (width, height) in [(160, 30), (100, 24), (80, 24)] {
        let screen = draw(&app, width, height);
        let text = screen.join("\n");
        let rule = screen
            .iter()
            .position(|l| l.starts_with("── ⏱ cron · 1"))
            .expect(&text);
        assert!(screen[..rule].iter().any(|l| l.contains("alpha")), "{text}");
        assert!(screen[..rule].iter().any(|l| l.contains("carol")), "{text}");
        assert!(screen[rule].contains("no clock"), "{text}");
        assert!(screen[rule + 1].contains("every 30m"), "{text}");
        assert!(screen[rule + 1].contains("someone"), "{text}");
    }
}

#[test]
fn a_member_with_an_active_job_shows_its_next_run_and_the_label_drops_first() {
    let app = squad_tab();
    let wide = draw(&app, 120, 24);
    let alpha = wide.iter().find(|l| l.contains("alpha")).unwrap();
    // The label's clock text depends on the real date; its exact form is
    // covered where time is injected (`cronboard::tests`).
    let label = alpha.split_once("⏱ ").expect(alpha).1;
    assert!(label.contains(':') && alpha.ends_with(label), "{alpha}");
    let carol = wide.iter().find(|l| l.contains("carol")).unwrap();
    assert!(!carol.contains('⏱'), "{carol}");
    // Too narrow for the label: it goes, the row's columns stay.
    let narrow = draw(&app, 30, 24);
    let alpha = narrow.iter().find(|l| l.contains("alpha")).unwrap();
    assert!(!alpha.contains('⏱'), "{alpha}");
}

#[test]
fn tab_enters_the_jobs_half_after_the_last_pane_and_returns() {
    let mut app = squad_tab();
    draw(&app, 100, 24);
    assert!(!app.jobs_focus);
    press(&mut app, KeyCode::Tab);
    assert!(
        app.jobs_focus,
        "Tab after the last pane enters the jobs half"
    );
    let screen = draw(&app, 100, 24);
    let text = screen.join("\n");
    // Focus expands the selected job in place.
    assert!(text.contains("message  job for u1"), "{text}");
    assert!(
        text.contains("next     ") && text.contains("tz Asia/Tokyo"),
        "{text}"
    );
    press(&mut app, KeyCode::Tab);
    assert!(!app.jobs_focus && app.focused_pane() == Some(Pane::Rows));
    assert!(!draw(&app, 100, 24).join("\n").contains("message  job"));
}

#[test]
fn scoped_keys_navigate_jobs_and_never_act_on_a_member_row() {
    let mut app = squad_tab();
    draw(&app, 100, 24);
    press(&mut app, KeyCode::Tab);
    draw(&app, 100, 24);
    // Enter goes to the job's owner, not the selected member row.
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        Effect::Act(crate::board::app::Request::Jump("someone".into()))
    );
    // The preset's member keys refuse while the jobs half has focus.
    for key in ['y', 't', 'r'] {
        assert_eq!(press(&mut app, KeyCode::Char(key)), Effect::None);
        assert!(
            app.notice
                .as_deref()
                .unwrap_or("")
                .contains("acts on a member row"),
            "{key}: {:?}",
            app.notice
        );
    }
    assert_eq!(app.selected, 0, "the member selection never moved");
}

#[test]
fn a_user_binding_wins_over_the_scoped_c_key_and_other_tabs_have_no_half() {
    let mut app = squad_tab();
    draw(&app, 100, 24);
    press(&mut app, KeyCode::Tab);
    app.view
        .as_mut()
        .unwrap()
        .configured_bindings
        .insert("c".into(), crate::action::Action::parse("refresh").unwrap());
    app.view
        .as_mut()
        .unwrap()
        .bindings
        .insert("c".into(), crate::action::Action::parse("refresh").unwrap());
    assert_eq!(press(&mut app, KeyCode::Char('c')), Effect::Refresh);
    let mut leads = squad_tab();
    leads.view.as_mut().unwrap().document["squad"]
        .as_object_mut()
        .unwrap()
        .remove("roomId");
    assert!(!draw(&leads, 100, 24).join("\n").contains("⏱ cron"));
}

#[test]
fn a_short_body_keeps_the_rule_only_and_a_pointer_press_focuses_the_half() {
    let app = squad_tab();
    let short = draw(&app, 100, 11);
    assert!(short.iter().any(|l| l.starts_with("── ⏱ cron · 1")));
    assert!(!short.join("\n").contains("every 30m"));
    let mut app = squad_tab();
    let screen = draw(&app, 100, 24);
    let job = screen.iter().position(|l| l.contains("every 30m")).unwrap();
    let click = |kind| MouseEvent {
        kind,
        column: 10,
        row: job as u16,
        modifiers: KeyModifiers::NONE,
    };
    app.mouse(
        click(MouseEventKind::Down(MouseButton::Left)),
        Instant::now(),
    );
    assert!(app.jobs_focus);
    app.mouse(
        MouseEvent {
            row: 2,
            ..click(MouseEventKind::Down(MouseButton::Left))
        },
        Instant::now(),
    );
    assert!(!app.jobs_focus, "a press elsewhere gives the focus back");
}

#[test]
fn a_failed_read_without_data_says_so_in_the_half() {
    let mut app = board(members());
    app.view.as_mut().unwrap().document["squad"]["roomId"] = json!("room");
    app.cron.replace(Err("storage unreachable".into()));
    let text = draw(&app, 100, 24).join("\n");
    assert!(
        text.contains("⏱ cron · 0 · ✗ storage unreachable"),
        "{text}"
    );
    assert!(text.contains("(jobs unavailable)"), "{text}");
}
