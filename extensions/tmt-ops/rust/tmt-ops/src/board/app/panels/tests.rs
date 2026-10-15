use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn board() -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.shell.tab = OpsTab::Overview;
    app
}
#[test]
fn preview_reads_override_but_cancel_uses_the_latest_acquisition_without_a_write() {
    let mut app = board();
    let expected = app.workspace_snapshot().unwrap().preferences.clone();
    assert_eq!(app.open_panels(), Effect::None);
    assert_eq!(app.panels_event(&key(KeyCode::Enter)), Some(Effect::None));
    assert!(
        !app.overview_panels()
            .shown(crate::config::OverviewPanel::Summary)
    );
    assert_eq!(app.workspace_snapshot().unwrap().preferences, expected);
    let mut fresh = OverviewPanels::default();
    fresh
        .set_shown(crate::config::OverviewPanel::Digest, false)
        .unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .overview_panels = Some(PanelSetting::configured(fresh.clone()));
    assert_eq!(app.panels_picker.as_ref().unwrap().expected, expected);
    assert_eq!(app.panels_event(&key(KeyCode::Esc)), Some(Effect::None));
    assert_eq!(app.overview_panels(), fresh);
    assert!(!app.panels_save_pending);
}
#[test]
fn pending_lane_is_exclusive_and_a_closed_picker_ignores_its_late_result() {
    let mut app = board();
    app.open_panels();
    // Select Save using the public plain-key path, not a fabricated painted hit.
    app.panels_event(&key(KeyCode::End));
    app.panels_event(&key(KeyCode::Up));
    assert!(matches!(
        app.panels_event(&key(KeyCode::Enter)),
        Some(Effect::Act(Request::WorkspacePreferences { .. }))
    ));
    assert!(app.panels_save_pending);
    assert_eq!(app.panels_event(&key(KeyCode::Enter)), Some(Effect::None));
    app.panels_event(&key(KeyCode::Esc));
    assert!(app.panels_picker.is_none());
    let old = app.notice.clone();
    assert_eq!(app.open_panels(), Effect::None);
    assert!(app.panels_picker.is_none());
    app.notice = old.clone();
    assert!(app.panels_finished(Ok("Panels saved.".into())));
    assert!(app.panels_picker.is_none());
    assert_eq!(app.notice, old);
    assert!(!app.panels_save_pending);
}
#[test]
fn failed_save_keeps_draft_and_opening_expected_and_success_closes() {
    let mut app = board();
    app.open_panels();
    app.panels_event(&key(KeyCode::Enter));
    let expected = app.panels_picker.as_ref().unwrap().expected.clone();
    let draft = app.overview_panels();
    app.panels_save_pending = true;
    app.panels_picker.as_mut().unwrap().start_save();
    assert!(app.panels_finished(Err(
        "Workspace preferences changed; reload before saving.".into()
    )));
    assert_eq!(app.overview_panels(), draft);
    assert_eq!(app.panels_picker.as_ref().unwrap().expected, expected);
    assert!(
        app.panels_picker
            .as_ref()
            .unwrap()
            .notice
            .as_deref()
            .unwrap()
            .contains("changed")
    );
    app.panels_save_pending = true;
    assert!(app.panels_finished(Ok("Panels saved.".into())));
    assert!(app.panels_picker.is_none());
    assert_eq!(app.notice.as_deref(), Some("Panels saved."));
}
#[test]
fn real_overlay_paints_and_owns_keys_while_closed_frames_are_unchanged() {
    use ratatui::{Terminal, backend::TestBackend};
    for tab in [OpsTab::Overview, OpsTab::Digest] {
        for width in [60, 100, 160] {
            let mut app = board();
            app.shell.tab = tab;
            let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let original = terminal.backend().buffer().clone();
            app.open_panels();
            assert!(app.modal_open() && app.reload_blocked());
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("Panels"), "{tab:?}/{width}");
            assert!(text.contains("Reset to default"));
            let before = app.panels_picker.as_ref().unwrap().next();
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert_ne!(app.panels_picker.as_ref().unwrap().next(), before);
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(app.panels_picker.is_none() && !app.modal_open());
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            assert_eq!(terminal.backend().buffer(), &original);
        }
    }
}
#[test]
fn theme_and_panels_cannot_open_together() {
    let mut app = board();
    app.open_panels();
    assert_eq!(
        app.perform(&crate::action::Action::parse("theme").unwrap()),
        Effect::None
    );
    assert!(app.theme_picker.is_none() && app.panels_picker.is_some());
    app.panels_event(&key(KeyCode::Esc));
    let path = std::env::temp_dir().join(format!(
        "ops-panels-theme-exclusive-{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "# fixture\n").unwrap();
    app.theme_picker = Some(
        crate::board::theme_picker::Picker::open(
            crate::config::Config::read(path.clone()).unwrap(),
            None,
        )
        .unwrap(),
    );
    assert_eq!(app.open_panels(), Effect::None);
    assert!(app.panels_picker.is_none() && app.theme_picker.is_some());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn mouse_toggle_and_move_use_fresh_painted_rows_and_match_plain_keys() {
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    };
    let mut mouse = board();
    mouse.open_panels();
    let mut keys = board();
    keys.open_panels();
    let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
    for (label, code) in [
        ("Summary counts", KeyCode::Enter),
        ("Move selected panel down", KeyCode::Char('d')),
    ] {
        terminal
            .draw(|frame| crate::board::view::render(frame, &mouse))
            .unwrap();
        let (row, column) = terminal
            .backend()
            .buffer()
            .content
            .chunks(100)
            .enumerate()
            .find_map(|(row, cells)| {
                let text: String = cells.iter().map(|cell| cell.symbol()).collect();
                text.find(label).map(|column| (row as u16, column as u16))
            })
            .expect("visible typed control");
        mouse.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        keys.key(KeyEvent::new(code, KeyModifiers::NONE));
        assert_eq!(
            mouse.panels_picker.as_ref().unwrap().next(),
            keys.panels_picker.as_ref().unwrap().next()
        );
    }
}
#[test]
fn hidden_chart_refusal_is_shared_by_shortcut_and_action_menu() {
    for tab in [OpsTab::Overview, OpsTab::Digest] {
        let mut app = board();
        app.shell.tab = tab;
        let prefs = &mut app
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences;
        if tab == OpsTab::Overview {
            let mut policy = OverviewPanels::default();
            policy
                .set_shown(crate::config::OverviewPanel::Usage, false)
                .unwrap();
            prefs.overview_panels = Some(PanelSetting::configured(policy));
        } else {
            let mut policy = DigestPanels::default();
            policy
                .set_shown(crate::config::DigestPanel::Chart, false)
                .unwrap();
            prefs.digest_panels = Some(PanelSetting::configured(policy));
        }
        let action = crate::action::Action::parse("usage-controls").unwrap();
        assert_eq!(app.perform(&action), Effect::None);
        assert_eq!(
            app.notice.as_deref(),
            Some("Usage chart is hidden by the panel policy.")
        );
        app.notice = None;
        app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert_eq!(
            app.notice.as_deref(),
            Some("Usage chart is hidden by the panel policy.")
        );
        assert!(!app.usage_controls);
    }
}

#[test]
fn pending_save_keeps_mouse_cancel_reachable_and_other_controls_disabled() {
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    };
    let mut app = board();
    app.open_panels();
    app.panels_event(&key(KeyCode::End));
    app.panels_event(&key(KeyCode::Up));
    assert!(matches!(
        app.panels_event(&key(KeyCode::Enter)),
        Some(Effect::Act(_))
    ));
    let next = app.panels_picker.as_ref().unwrap().next();
    let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let (row, column) = terminal
        .backend()
        .buffer()
        .content
        .chunks(100)
        .enumerate()
        .find_map(|(row, cells)| {
            let text: String = cells.iter().map(|cell| cell.symbol()).collect();
            text.find("Cancel")
                .map(|column| (row as u16, column as u16))
        })
        .unwrap();
    assert_eq!(app.panels_event(&key(KeyCode::Enter)), Some(Effect::None));
    assert_eq!(app.panels_picker.as_ref().unwrap().next(), next);
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert!(app.panels_picker.is_none());
    assert!(app.panels_save_pending);
    let notice = app.notice.clone();
    assert!(app.panels_finished(Err("stale".into())));
    assert_eq!(app.notice, notice);
    assert!(app.panels_picker.is_none());
}

fn overview_with_hidden_conversation() -> App {
    let mut app = board();
    let mut policy = OverviewPanels::default();
    policy.set_side_shown(false).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .overview_panels = Some(PanelSetting::configured(policy));
    app
}

fn paint(app: &mut App, width: u16, height: u16) {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
}

#[test]
fn explicit_message_reveals_the_same_selected_conversation_and_escape_keeps_its_draft() {
    use crate::config::Pane;
    for width in [60, 100, 139, 140, 160] {
        let mut app = overview_with_hidden_conversation();
        paint(&mut app, width, 36);
        let selected = app.workspace.selected.clone();
        assert!(app.collapsed_panes().contains(&Pane::Replies));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.shell.tab, OpsTab::Overview);
        assert_eq!(app.focused_pane(), Some(Pane::Replies));
        paint(&mut app, width, 36);
        let area = app.conversation.borrow().area;
        let pane = app.conversation.borrow().pane;
        assert_eq!(pane, app.overview_placement(area).conversation);
        assert!(pane.is_some());
        assert!(app.input_band.get().is_some());
        app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "x");
        app.invalidate_frame_geometry();
        app.set_body_width(60);
        paint(&mut app, 60, 18);
        assert_eq!(app.focused_pane(), Some(Pane::Replies));
        assert_eq!(app.workspace.selected, selected);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        paint(&mut app, 60, 18);
        assert_eq!(app.focused_pane(), Some(Pane::Rows));
        assert!(app.conversation.borrow().pane.is_none());
        assert_eq!(app.workspace.selected, selected);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "x");
        assert_eq!(app.workspace.selected, selected);
    }
}

#[test]
fn tab_pointer_and_refresh_do_not_reveal_a_policy_hidden_conversation() {
    use crate::config::Pane;
    let mut app = overview_with_hidden_conversation();
    paint(&mut app, 160, 36);
    let selected = app.workspace.selected.clone();
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    // Where the default right pane would have been is now part of the list.
    app.focus_at(159, 20);
    app.invalidate_frame_geometry();
    app.set_body_width(160);
    app.restore_focus();
    paint(&mut app, 160, 36);
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    assert!(app.conversation.borrow().pane.is_none());
    assert_eq!(app.workspace.selected, selected);
}

#[test]
fn revealed_conversation_survives_preview_save_and_uses_the_existing_busy_refusal() {
    let mut app = overview_with_hidden_conversation();
    paint(&mut app, 160, 36);
    let selected = app.workspace.selected.clone();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.in_flight = 1;
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "x");
    app.open_panels();
    assert!(app.panels_picker.is_none());
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.focused_pane(), Some(crate::config::Pane::Replies));
    app.in_flight = 0;
    app.open_panels();
    // Conversation is after the six ordered panel rows, selected by plain keys.
    for _ in 0..6 {
        app.panels_event(&key(KeyCode::Down));
    }
    app.panels_event(&key(KeyCode::Enter));
    assert!(app.overview_panels().side_shown());
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "x");
    paint(&mut app, 100, 18);
    app.panels_event(&key(KeyCode::End));
    app.panels_event(&key(KeyCode::Up));
    let Some(Effect::Act(Request::WorkspacePreferences { next, .. })) =
        app.panels_event(&key(KeyCode::Enter))
    else {
        panic!("Save uses the existing preference lane")
    };
    assert!(
        next.overview_panels
            .as_ref()
            .unwrap()
            .effective()
            .side_shown()
    );
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences = next;
    app.panels_finished(Ok("Panels saved.".into()));
    // Shown compact Conversation follows the pre-existing Agents fallback;
    // the same mounted input and UUID draft travel with that placement.
    app.set_body_width(100);
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "x");
    assert_eq!(app.workspace.selected, selected);
}

mod capture;

#[test]
fn hidden_monitor_cannot_retain_row_focus_or_a_selected_row_action() {
    for width in [60, 100, 140, 160] {
        let mut app = board();
        let selected = app.workspace.selected.clone();
        let mut policy = OverviewPanels::default();
        policy
            .set_shown(crate::config::OverviewPanel::Monitor, false)
            .unwrap();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences
            .overview_panels = Some(PanelSetting::configured(policy));
        paint(&mut app, width, 36);
        assert_eq!(app.focused_pane(), Some(crate::config::Pane::Replies));
        assert!(app.collapsed_panes().contains(&crate::config::Pane::Rows));
        assert!(!app.workspace_selection_visible());
        assert!(app.hits.borrow().iter().all(|hit| hit.row().is_none()));
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focused_pane(), Some(crate::config::Pane::Replies));
        assert_eq!(app.workspace.selected, selected);
    }
}
#[test]
fn hiding_conversation_after_tab_focus_does_not_create_an_explicit_message_reveal() {
    let mut app = board();
    paint(&mut app, 160, 36);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focused_pane(), Some(crate::config::Pane::Replies));
    assert!(app.input.is_none());
    let selected = app.workspace.selected.clone();
    let mut policy = OverviewPanels::default();
    policy.set_side_shown(false).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .overview_panels = Some(PanelSetting::configured(policy));
    app.invalidate_frame_geometry();
    app.restore_focus();
    paint(&mut app, 160, 36);
    assert_eq!(app.focused_pane(), Some(crate::config::Pane::Rows));
    assert!(app.conversation.borrow().pane.is_none());
    assert_eq!(app.workspace.selected, selected);
}

#[test]
fn workspace_pointer_uses_the_same_policy_rectangles_as_overview_paint() {
    use crate::{
        board::workspace::OpsTarget,
        config::{OverviewPanel, Pane},
    };
    use ratatui::{Terminal, backend::TestBackend};
    for side_shown in [false, true] {
        let mut app = board();
        app.workspace.selected = Some(OpsTarget::Member {
            room_id: Some(crate::board::app::workspace::tests::ROOM.into()),
            identity_id: crate::board::app::workspace::tests::MEMBER.into(),
        });
        let mut policy = OverviewPanels::default();
        if side_shown {
            policy.set_shown(OverviewPanel::Monitor, false).unwrap();
        } else {
            policy.set_side_shown(false).unwrap();
        }
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences
            .overview_panels = Some(PanelSetting::configured(policy));
        // At compact width a hidden monitor leaves Conversation as the only
        // interactive pane; at wide width disabling Conversation frees its space.
        let width = if side_shown { 100 } else { 160 };
        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
        let area = app.conversation.borrow().area;
        let expected = if side_shown {
            Pane::Replies
        } else {
            Pane::Rows
        };
        assert_eq!(
            app.pointer_pane(area.right() - 2, area.y + 2),
            Some(expected)
        );
        assert_eq!(app.pointer_pane(area.x + 2, area.y + 2), Some(expected));
    }
}
