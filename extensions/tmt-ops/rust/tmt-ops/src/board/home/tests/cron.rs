//! The current workspace opens scoped timers without restoring the old HOME cursor.
use crate::board::{
    app::workspace::tests::{ROOM, board},
    app::{App, Effect},
    cronboard::{TEST_NOW, test_cron, test_view},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use tmt_ops::cron::ClockStatus;

fn with_jobs(app: &mut App) {
    let mut job = test_view("worker", "merge queue sweep", Some(TEST_NOW + 3_600_000));
    job.job.squad = "product".into();
    job.job.room_id = ROOM.into();
    app.cron
        .replace(Ok(test_cron(vec![job], ClockStatus::NoClock)));
}
fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
#[test]
fn scoped_timer_list_opens_after_a_read_and_cancel_keeps_the_exact_target() {
    let mut app = board();
    let selected = app.workspace.selected.clone();
    assert_eq!(press(&mut app, KeyCode::Char('c')), Effect::None);
    assert!(app.cron_list.is_none());
    with_jobs(&mut app);
    press(&mut app, KeyCode::Char('c'));
    assert!(app.cron_list.is_some());
    let rendered = screen(&app);
    assert!(rendered.contains("merge queue sweep"), "{rendered}");
    press(&mut app, KeyCode::Char('s'));
    press(&mut app, KeyCode::Char('a'));
    assert!(app.input.is_none() && app.menu.is_none());
    press(&mut app, KeyCode::Esc);
    assert!(app.cron_list.is_none());
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.current.as_deref(), Some(crate::board::ALL));
}
#[test]
fn configured_timer_key_keeps_priority_over_workspace_defaults() {
    let mut app = board();
    with_jobs(&mut app);
    let keys = &mut app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .keys;
    keys.get_mut(&crate::config::OpsKeyScope::Member)
        .unwrap()
        .insert("c".into(), crate::action::Action::parse("refresh").unwrap());
    assert_eq!(press(&mut app, KeyCode::Char('c')), Effect::Refresh);
    assert!(app.cron_list.is_none());
}
#[test]
fn failed_timer_reads_are_inspectable_without_inventing_an_empty_job_list() {
    let mut app = board();
    app.cron.replace(Err("storage unreachable".into()));
    press(&mut app, KeyCode::Char('c'));
    assert!(app.cron_list.is_some());
    let rendered = screen(&app);
    assert!(rendered.contains("storage unreachable"), "{rendered}");
    assert_eq!(app.cron.failure.as_deref(), Some("storage unreachable"));
}
