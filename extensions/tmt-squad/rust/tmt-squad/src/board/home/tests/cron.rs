use super::interaction::{board, press, waiting};
use crate::board::{
    app::{App, Effect},
    cronboard::{TEST_NOW, test_cron, test_view},
    home::paint,
};
use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyCode::*, layout::Rect};
use tmt_squad::cron::ClockStatus;

fn with_jobs(app: &mut App) {
    let mut job = test_view("lead-a", "merge queue sweep", Some(TEST_NOW + 3_600_000));
    job.job.squad = "a".into();
    app.cron
        .replace(Ok(test_cron(vec![job], ClockStatus::NoClock)));
}

fn screen(app: &App, width: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
    terminal
        .draw(|frame| paint::render_at(frame, app, Rect::new(0, 0, width, 24), 100))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(width as usize)
        .map(|cells| {
            cells
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

#[test]
fn the_cron_line_sits_between_attention_and_squads_in_one_cursor_order() {
    let mut app = board(&[("a", waiting())]);
    assert!(
        !screen(&app, 120).join("\n").contains("⑤"),
        "no read yet: no line"
    );
    with_jobs(&mut app);
    for width in [160, 100, 80] {
        let lines = screen(&app, width);
        let at = |needle: &str| lines.iter().position(|line| line.contains(needle)).unwrap();
        assert!(at("✗ blocked") < at("⑤ ⏱ 1"), "{lines:#?}");
        assert!(at("⑤ ⏱ 1") < at("③ squads"), "{lines:#?}");
        assert!(lines[at("⑤ ⏱ 1")].ends_with("c list"), "{lines:#?}");
    }
    let sections = |app: &App| {
        app.home_entries()
            .iter()
            .map(|entry| entry.target.section.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(sections(&app), ["needs-you", "blocked", "cron", "squads"]);
    press(&mut app, Tab);
    assert_eq!(app.home_target.as_ref().unwrap().section, "cron");
    press(&mut app, Tab);
    assert_eq!(app.home_target.as_ref().unwrap().section, "squads");
    press(&mut app, BackTab);
    assert_eq!(app.home_target.as_ref().unwrap().section, "cron");
    // A search hides the cron line like any non-matching row.
    app.search = "worker".into();
    assert!(!sections(&app).contains(&"cron".to_owned()));
}

#[test]
fn enter_and_c_open_the_list_and_esc_restores_the_cursor() {
    let mut app = board(&[("a", waiting())]);
    assert_eq!(press(&mut app, Char('c')), Effect::None);
    assert!(app.cron_list.is_none(), "nothing read yet");
    with_jobs(&mut app);
    press(&mut app, Tab);
    assert_eq!(press(&mut app, Enter), Effect::None);
    assert!(app.cron_list.is_some());
    // The modal swallows keys that would act on the board underneath.
    press(&mut app, Char('s'));
    press(&mut app, Char('a'));
    assert!(app.switcher.is_none() && app.input.is_none() && app.menu.is_none());
    press(&mut app, Esc);
    assert!(app.cron_list.is_none());
    assert_eq!(app.home_target.as_ref().unwrap().section, "cron");
    press(&mut app, Char('c'));
    assert!(app.cron_list.is_some());
    assert_eq!(press(&mut app, Enter), Effect::Load("a".into()));
    assert!(app.cron_list.is_none());
}

#[test]
fn a_user_binding_on_c_keeps_priority() {
    let mut app = board(&[("a", waiting())]);
    with_jobs(&mut app);
    app.view
        .as_mut()
        .unwrap()
        .bindings
        .insert("c".into(), crate::action::Action::parse("refresh").unwrap());
    assert_eq!(press(&mut app, Char('c')), Effect::Refresh);
    assert!(app.cron_list.is_none());
}

#[test]
fn a_failed_read_is_a_selectable_line_and_the_list_says_why() {
    let mut app = board(&[("a", waiting())]);
    app.cron.replace(Err("storage unreachable".into()));
    let lines = screen(&app, 120);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("⑤ ⏱ ✗ cron jobs unavailable: storage unreachable")),
        "{lines:#?}"
    );
    press(&mut app, Tab);
    press(&mut app, Enter);
    assert!(app.cron_list.is_some());
}
