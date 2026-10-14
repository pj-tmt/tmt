use super::*;

fn board() -> crate::board::app::App {
    let mut app = crate::board::app::workspace::tests::board();
    app.shell.tab = crate::board::shell::OpsTab::Overview;
    app
}
fn paint(app: &mut crate::board::app::App, width: u16) {
    app.set_body_width(width);
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 44)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
}
fn click(app: &mut crate::board::app::App, x: u16, y: u16) {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
}

#[test]
fn global_timers_need_no_uuid_and_new_stays_refused() {
    use crate::board::app::HitTarget;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let _no_commands = crate::runner::forbid_commands();
    let mut app = board();
    app.workspace.selected = None;
    paint(&mut app, 160);
    let hit = *app
        .hits
        .borrow()
        .iter()
        .find(|hit| hit.target == HitTarget::AllRoomTimers)
        .unwrap();
    click(&mut app, hit.x, hit.y);
    assert!(app.cron_list.is_none());
    assert_eq!(
        app.notice.as_deref(),
        Some("Timers have not been read yet.")
    );
    app.cron.replace(Ok(crate::board::cronboard::test_cron(
        vec![],
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    paint(&mut app, 160);
    click(&mut app, hit.x, hit.y);
    let list = app
        .cron_list
        .as_ref()
        .expect("global hit opens existing list");
    assert!(list.scope.is_none() && list.owner.is_none());
    assert!(app.workspace.selected.is_none());
    app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|notice| notice.contains("explicit squad or job"))
    );
    assert!(app.input.is_none());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    let list = app
        .cron_list
        .as_ref()
        .expect("c with no selected scope opens same list");
    assert!(list.scope.is_none() && list.owner.is_none());
    assert!(app.workspace.selected.is_none());
}

#[test]
fn global_hit_is_replaced_on_repaint_and_does_not_retarget_hidden_selection() {
    use crate::board::app::HitTarget;
    let mut app = board();
    app.cron.replace(Ok(crate::board::cronboard::test_cron(
        vec![],
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    app.search = "no matching agent".into();
    app.workspace_reconcile();
    let selected = app.workspace.selected.clone();
    paint(&mut app, 160);
    let hit = *app
        .hits
        .borrow()
        .iter()
        .find(|hit| hit.target == HitTarget::AllRoomTimers)
        .unwrap();
    click(&mut app, hit.x, hit.y);
    assert!(
        app.cron_list
            .as_ref()
            .is_some_and(|list| list.scope.is_none() && list.owner.is_none())
    );
    assert_eq!(app.workspace.selected, selected);
    app.cron_list = None;
    paint(&mut app, 20);
    assert!(
        app.hits
            .borrow()
            .iter()
            .all(|hit| hit.target != HitTarget::AllRoomTimers)
    );
    click(&mut app, hit.x, hit.y);
    assert!(
        app.cron_list.is_none(),
        "removed control cannot retain authority"
    );
}

#[test]
fn keyboard_timers_never_widens_a_hidden_missing_or_other_tab_scope() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = board();
    app.cron.replace(Ok(crate::board::cronboard::test_cron(
        vec![],
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    app.search = "no matching agent".into();
    app.workspace_reconcile();
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.cron_list.is_none());
    assert_eq!(
        app.notice.as_deref(),
        Some("Select a visible timer scope explicitly.")
    );
    app.workspace.selected = Some(crate::board::workspace::OpsTarget::Member {
        room_id: None,
        identity_id: "removed".into(),
    });
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.cron_list.is_none());
    for tab in [
        crate::board::shell::OpsTab::Agents,
        crate::board::shell::OpsTab::Digest,
    ] {
        app.shell.tab = tab;
        app.workspace.selected = None;
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(app.cron_list.is_none());
    }
}

#[test]
fn every_panel_is_disjoint_and_short_heights_keep_the_monitor() {
    for width in [0, 1, 60, 100, 160] {
        for height in [0, 1, 8, 18, 24, 36, 44] {
            let area = Rect::new(2, 3, width, height);
            let panels = panels(area, 2);
            for (_, panel) in &panels {
                assert_eq!(panel.intersection(area), *panel);
            }
            assert!(
                panels
                    .windows(2)
                    .all(|pair| pair[0].1.bottom() <= pair[1].1.y)
            );
            if !area.is_empty() {
                let table = panels
                    .iter()
                    .find(|(panel, _)| *panel == Panel::Table)
                    .unwrap()
                    .1;
                assert!(
                    table.height >= height.min(6),
                    "{width}x{height}: {panels:?}"
                );
                assert_eq!(
                    panels.iter().map(|(_, area)| area.height).sum::<u16>(),
                    height
                );
            } else {
                assert!(panels.is_empty());
            }
        }
    }
}
