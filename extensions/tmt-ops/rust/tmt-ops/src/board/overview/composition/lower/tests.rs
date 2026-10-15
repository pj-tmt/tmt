use super::*;
use crate::board::{
    app::{
        Effect,
        workspace::tests::{board, observe_waiting_member},
    },
    cronboard::{test_cron, test_views},
    shell::OpsTab,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
fn app() -> App {
    let mut app = board();
    observe_waiting_member(&mut app);
    app.shell.tab = OpsTab::Overview;
    app.cron.replace(Ok(test_cron(
        test_views(5, "team", "room"),
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    app
}
fn paint(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn narrow_preview_and_default_key_share_explicit_all_room_scope() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = app();
    let selected = app.workspace.selected.clone();
    let shown = paint(&mut app, 103, 51);
    assert!(shown.contains("+2 more · c timers"), "{shown}");
    let lines: Vec<_> = shown.lines().collect();
    let timer = lines
        .iter()
        .position(|line| line.contains("── Timers"))
        .unwrap();
    assert_eq!(lines[timer].chars().nth(62), Some('┬'));
    assert!(lines[timer].contains("Digest"));
    let mode = lines
        .iter()
        .find(|line| line.contains("Auto · timed · Off"))
        .unwrap();
    assert!(mode.ends_with("— "));
    let hit = *app
        .hits
        .borrow()
        .iter()
        .find(|hit| hit.target == HitTarget::AllRoomTimers)
        .unwrap();
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    let list = app.cron_list.as_ref().unwrap();
    assert!(list.scope.is_none() && list.owner.is_none());
    assert_eq!(app.workspace.selected, selected);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    let list = app.cron_list.as_ref().unwrap();
    assert!(list.scope.is_none() && list.owner.is_none());
    assert_eq!(app.workspace.selected, selected);
}
#[test]
fn wide_has_no_interim_key_value_band_and_resize_revokes_its_hit() {
    let mut app = app();
    paint(&mut app, 103, 51);
    let old = *app
        .hits
        .borrow()
        .iter()
        .find(|hit| hit.target == HitTarget::AllRoomTimers)
        .unwrap();
    let wide = paint(&mut app, 148, 51);
    assert!(!wide.contains("Auto · timed · Off"));
    assert!(
        app.hits
            .borrow()
            .iter()
            .all(|hit| hit.target != HitTarget::AllRoomTimers)
    );
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: old.x,
            row: old.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert!(app.cron_list.is_none());
}
#[test]
fn no_unacquired_preview_hit_or_command_and_short_rows_remain_bounded() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = app();
    app.cron = Default::default();
    let shown = paint(&mut app, 103, 51);
    assert!(shown.contains("Timers unavailable"));
    assert!(
        app.hits
            .borrow()
            .iter()
            .all(|hit| hit.target != HitTarget::AllRoomTimers)
    );
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)),
        Effect::None
    );
    assert_eq!(
        app.notice.as_deref(),
        Some("Timers have not been read yet.")
    );
    for (width, height) in [(60, 8), (60, 18), (103, 8), (103, 18), (148, 8)] {
        paint(&mut app, width, height);
        assert!(
            app.hits
                .borrow()
                .iter()
                .all(|hit| hit.x + hit.width <= width && hit.y < height)
        );
    }
}

fn configured(app: &mut App, text: &str) {
    let path = std::env::temp_dir().join(format!(
        "ops-timer-keys-{}.toml",
        crate::id::new_v4().unwrap()
    ));
    std::fs::write(&path, text).unwrap();
    let config = crate::config::Config::read(path.clone()).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .configure(&config)
        .unwrap();
    std::fs::remove_file(path).unwrap();
    app.overview_rows.borrow_mut().clear();
    app.invalidate_frame_geometry();
}
#[test]
fn authored_scoped_timer_and_host_keys_are_never_shadowed_by_the_preview() {
    use crate::board::app::{Request, workspace::tests::MEMBER};
    let _no_commands = crate::runner::forbid_commands();
    let mut app = app();
    configured(
        &mut app,
        r#"[bind]
c = "timers"
y = "all-timers"
"#,
    );
    let shown = paint(&mut app, 103, 51);
    assert!(shown.contains("+2 more · y timers"), "{shown}");
    assert!(!shown.contains("c timers"));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert_eq!(
        app.cron_list.as_ref().unwrap().owner.as_deref(),
        Some(MEMBER)
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    let list = app.cron_list.as_ref().unwrap();
    assert!(list.scope.is_none() && list.owner.is_none());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    configured(
        &mut app,
        r#"[bind]
c = "timers"
[bind.tmux]
c = "jump"
"#,
    );
    let shown = paint(&mut app, 103, 51);
    assert!(!shown.contains("c timers"));
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)),
        Effect::Act(Request::Jump(MEMBER.into()))
    );
}

#[test]
fn hiding_compact_digest_preserves_the_timer_source_and_removes_the_divider() {
    let mut app = app();
    let selected = app.workspace.selected.clone();
    assert!(paint(&mut app, 103, 51).contains("Auto · timed · Off"));
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut policy = snapshot
        .preferences
        .overview_panels
        .as_ref()
        .map(crate::config::PanelSetting::effective)
        .unwrap_or_default();
    policy
        .set_shown(crate::config::OverviewPanel::Digest, false)
        .unwrap();
    snapshot.preferences.overview_panels = Some(crate::config::PanelSetting::configured(policy));
    app.invalidate_frame_geometry();
    let shown = paint(&mut app, 103, 51);
    assert!(!shown.contains("Auto · timed · Off") && !shown.contains('┬'));
    assert!(shown.contains("+2 more · c timers"));
    assert_eq!(
        app.hits
            .borrow()
            .iter()
            .filter(|hit| hit.target == HitTarget::AllRoomTimers)
            .count(),
        1
    );
    assert_eq!(app.workspace.selected, selected);
}
