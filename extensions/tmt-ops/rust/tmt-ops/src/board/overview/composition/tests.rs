use crate::board::{
    app::workspace::tests::{MEMBER, ROOM},
    app::{App, Effect, HitTarget},
    shell::OpsTab,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
fn board() -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.shell.tab = OpsTab::Overview;
    app
}
fn waiting(app: &mut App) {
    app.overview_view = crate::board::overview::monitor::View::Agents;
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.roster.members.get_mut(MEMBER).unwrap().waiting = true;
    snapshot.roster.squads[0]
        .member_states
        .get_mut(MEMBER)
        .unwrap()
        .waiting = true;
    snapshot
        .rows
        .get_mut(&crate::board::workspace::OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        })
        .unwrap()["pending"] = serde_json::json!("Approve this change?");
    snapshot.shell_counts = snapshot.roster.shell_counts();
    app.overview_rows.borrow_mut().clear();
    app.workspace_reconcile();
}
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
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
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
#[test]
fn global_timers_need_no_uuid_and_new_stays_refused() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = board();
    app.workspace.selected = None;
    paint(&mut app, 103, 51);
    assert!(
        app.hits
            .borrow()
            .iter()
            .all(|hit| hit.target != HitTarget::AllRoomTimers)
    );
    key(&mut app, KeyCode::Char('c'));
    assert!(app.cron_list.is_none());
    assert_eq!(
        app.notice.as_deref(),
        Some("Timers have not been read yet.")
    );
    app.cron.replace(Ok(crate::board::cronboard::test_cron(
        vec![],
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    key(&mut app, KeyCode::Char('c'));
    let list = app
        .cron_list
        .as_ref()
        .expect("explicit global key opens the existing list");
    assert!(list.scope.is_none() && list.owner.is_none());
    assert!(app.workspace.selected.is_none());
    key(&mut app, KeyCode::Char('n'));
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|notice| notice.contains("explicit squad or job"))
    );
    assert!(app.input.is_none());
}
#[test]
fn removed_count_cells_cannot_retain_their_old_timer_authority() {
    let mut app = board();
    app.shell.tab = OpsTab::Agents;
    app.cron.replace(Ok(crate::board::cronboard::test_cron(
        vec![],
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    paint(&mut app, 160, 36);
    let old = app
        .cron_scope_hits
        .borrow()
        .first()
        .cloned()
        .expect("positive control: an actual count hit");
    app.shell.tab = OpsTab::Overview;
    paint(&mut app, 100, 36);
    assert!(app.cron_scope_hits.borrow().is_empty());
    let selected = app.workspace.selected.clone();
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: old.area.x.min(99),
            row: old.area.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert!(app.cron_list.is_none());
    assert_eq!(app.workspace.selected, selected);
}
#[test]
fn one_primary_viewport_keeps_waiting_targets_and_hits_bounded_at_short_heights() {
    for width in [60, 103, 148, 160] {
        for height in [8, 18, 24, 36, 51] {
            let mut app = board();
            waiting(&mut app);
            paint(&mut app, width, height);
            let hits: Vec<_> = app
                .hits
                .borrow()
                .iter()
                .filter(|hit| hit.row().is_some())
                .copied()
                .collect();
            assert_eq!(hits.len(), 1, "{width}x{height} retains its waiting target");
            for hit in hits {
                assert_eq!(hit.row(), Some(0));
                assert!(hit.x + hit.width <= width && hit.y < height);
            }
        }
    }
}
#[test]
fn hiding_the_primary_container_clears_its_previous_hit_owner() {
    let mut app = board();
    waiting(&mut app);
    paint(&mut app, 160, 36);
    assert!(app.hits.borrow().iter().any(|hit| hit.row().is_some()));
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut policy = snapshot
        .preferences
        .overview_panels
        .as_ref()
        .map(|setting| setting.effective())
        .unwrap_or_default();
    policy
        .set_shown(crate::config::OverviewPanel::Monitor, false)
        .unwrap();
    snapshot.preferences.overview_panels = Some(crate::config::PanelSetting::configured(policy));
    app.invalidate_frame_geometry();
    paint(&mut app, 160, 36);
    assert!(app.hits.borrow().iter().all(|hit| hit.row().is_none()));
}
#[test]
fn eight_rows_keep_view_details_and_usage_keys_without_a_table_header() {
    for width in [60, 100, 160] {
        let mut app = board();
        waiting(&mut app);
        let selected = app.workspace.selected.clone();
        key(&mut app, KeyCode::Char('u'));
        assert!(!app.usage_controls);
        app.notice = None;
        let text = paint(&mut app, width, 8);
        assert!(!text.contains("Subscriptions") && !text.contains("Digest status"));
        assert!(app.hits.borrow().iter().any(|hit| hit.row() == Some(0)));
        key(&mut app, KeyCode::Char('f'));
        assert!(app.overview_view == crate::board::overview::monitor::View::Squads);
        key(&mut app, KeyCode::Char('f'));
        assert!(app.overview_view == crate::board::overview::monitor::View::Agents);
        assert_eq!(app.workspace.selected, selected);
        key(&mut app, KeyCode::Char('e'));
        assert!(app.monitor_details.is_some());
        key(&mut app, KeyCode::Esc);
        assert!(app.monitor_details.is_none());
        app.invalidate_frame_geometry();
        key(&mut app, KeyCode::Char('u'));
        assert!(
            !app.usage_controls,
            "invalidated geometry admits no controls"
        );
        paint(&mut app, width, 8);
        key(&mut app, KeyCode::Char('u'));
        assert!(app.usage_controls);
        let modal = paint(&mut app, width, 8);
        assert!(
            modal.contains("Range") && modal.contains("1h") && modal.contains("6h"),
            "short modal remains operable: {modal}"
        );
        key(&mut app, KeyCode::Esc);
        assert!(!app.usage_controls);
        assert_eq!(app.workspace.selected, selected);
    }
}
#[test]
fn authored_selected_scope_timers_never_widens_a_hidden_missing_or_other_tab_scope() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = board();
    // Acquired authored bindings retain the selected-scope action; Overview's
    // new default is a separate, explicit all-room action.
    for keys in app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .keys
        .values_mut()
    {
        keys.insert("c".into(), crate::action::Action::parse("timers").unwrap());
    }

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
