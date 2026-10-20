//! A click outside a peek window closes it and lands on what was under it; drafts and dialogs stay.
use crate::board::{
    app::workspace::tests::{ROOM, board},
    app::{App, Effect},
    cronboard::{TEST_NOW, test_cron, test_view},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
use std::time::Instant;
use tmt_ops::cron::ClockStatus;

fn with_jobs(app: &mut App) {
    let mut job = test_view("worker", "merge queue sweep", Some(TEST_NOW + 3_600_000));
    job.job.squad = "product".into();
    job.job.room_id = ROOM.into();
    job.job.owner_id = Some(crate::board::app::workspace::tests::MEMBER.into());
    app.cron
        .replace(Ok(test_cron(vec![job], ClockStatus::NoClock)));
}
fn paint(app: &App) {
    let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
}
fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) -> Effect {
    app.mouse(
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        Instant::now(),
    )
}
fn click(app: &mut App, column: u16, row: u16) -> Effect {
    mouse(app, MouseEventKind::Down(MouseButton::Left), column, row)
}

/// The painted position of the first row hit that is not the selected one, outside any window.
fn other_row(app: &App) -> (u16, u16, usize) {
    app.hits
        .borrow()
        .iter()
        .find_map(|hit| {
            hit.row()
                .filter(|row| *row != app.selected)
                .map(|row| (hit.x + 2, hit.y, row))
        })
        .expect("a second painted row")
}
fn open_timers(app: &mut App) {
    with_jobs(app);
    press(app, KeyCode::Char('c'));
    assert!(app.cron_list.is_some());
    paint(app);
}

#[test]
fn an_outside_press_closes_the_timers_window_and_selects_the_row_under_it_at_once() {
    let mut app = board();
    open_timers(&mut app);
    let (x, y, row) = other_row(&app);
    click(&mut app, x, y);
    assert!(app.cron_list.is_none());
    assert_eq!(app.selected, row);
}

#[test]
fn a_press_inside_the_timers_window_keeps_it_and_a_wheel_outside_does_not_close_it() {
    let mut app = board();
    open_timers(&mut app);
    let outer = app.cron_list.as_ref().unwrap().modal_outer().unwrap();
    let before = app.selected;
    click(&mut app, outer.x + 2, outer.y + 2);
    assert!(app.cron_list.is_some());
    let (x, y, _) = other_row(&app);
    mouse(&mut app, MouseEventKind::ScrollDown, x, y);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), x, y);
    assert!(app.cron_list.is_some());
    assert_eq!(app.selected, before);
}

#[test]
fn esc_and_q_still_close_the_timers_window() {
    for code in [KeyCode::Esc, KeyCode::Char('q')] {
        let mut app = board();
        open_timers(&mut app);
        press(&mut app, code);
        assert!(app.cron_list.is_none(), "{code:?}");
    }
}

#[test]
fn a_window_with_no_fresh_paint_never_dismisses() {
    let mut app = board();
    open_timers(&mut app);
    let (x, y, _) = other_row(&app);
    let before = app.selected;
    app.peek_areas.clear();
    click(&mut app, x, y);
    assert!(app.cron_list.is_some());
    assert_eq!(app.selected, before);
}

#[test]
fn an_open_draft_keeps_the_window_on_an_outside_press() {
    let mut app = board();
    open_timers(&mut app);
    app.ask(
        String::new(),
        crate::board::app::Compose::Talk {
            to: "worker".into(),
        },
        crate::board::app::workspace::tests::ROOM.into(),
    );
    assert!(app.input.is_some());
    let (x, y, _) = other_row(&app);
    click(&mut app, x, y);
    assert!(app.cron_list.is_some());
}

#[test]
fn the_help_window_closes_on_an_outside_press() {
    let mut app = board();
    app.help = true;
    paint(&app);
    // The sheet fills the body; the header above it is outside.
    click(&mut app, 0, 0);
    assert!(!app.help);
}

#[test]
fn an_unanchored_menu_closes_on_an_outside_press_and_the_press_lands() {
    let mut app = board();
    app.context_menu();
    assert!(app.menu.is_some());
    paint(&app);
    let (x, y, row) = other_row(&app);
    click(&mut app, x, y);
    assert!(app.menu.is_none());
    assert_eq!(app.selected, row);
}

#[test]
fn the_agent_details_and_subscription_windows_close_on_an_outside_press() {
    let mut app = board();
    assert_eq!(app.overview_details(), Effect::None);
    assert!(app.monitor_details.is_some());
    paint(&app);
    click(&mut app, 0, 0);
    assert!(app.monitor_details.is_none());

    app.shell.tab = crate::board::shell::OpsTab::Overview;
    assert_eq!(app.open_quota_details(), Effect::None);
    assert!(app.quota_details.is_some());
    paint(&app);
    click(&mut app, 0, 0);
    assert!(app.quota_details.is_none());
}
