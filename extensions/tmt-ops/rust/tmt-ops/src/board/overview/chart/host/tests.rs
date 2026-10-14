use super::*;

#[test]
fn invalid_control_change_preserves_grid_and_inspected_bucket() {
    let mut state = State::default();
    state.apply(Target::Range(15)).unwrap();
    state.bar = 2;
    assert!(state.apply(Target::Each(30)).is_err());
    assert_eq!(
        state.selection,
        Selection {
            each_minutes: 5,
            range_minutes: 15
        }
    );
    assert_eq!(state.bar, 2);
    state.apply(Target::Each(15)).unwrap();
    assert_eq!(state.bar, 0);
}

#[test]
fn source_and_geometry_invalidation_have_distinct_lifetimes() {
    let mut state = State::default();
    state.set_focus(Focus::Range);
    state.visible = true;
    state.hits.push(Hit {
        area: Rect::new(1, 2, 3, 1),
        target: Target::Range(15),
    });
    state.observed = Some(Rc::new(series::project(
        [],
        Grid::new(300_000, 3_600_000, 3_600_000).unwrap(),
        None,
    )));
    state.invalidate_geometry();
    assert!(state.hits.is_empty());
    assert!(state.observed.is_some());
    assert!(state.focused());
    state.invalidate_data();
    assert!(state.observed.is_none());
    assert_eq!(state.selection, Selection::default());
}

use crate::board::app::workspace::tests::{MEMBER, board};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
fn paint(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}
fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn click(app: &mut App, area: Rect) {
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
}
fn text(buffer: &Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn current_controls_and_keyboard_share_grid_without_recipient_or_draft_changes() {
    crate::status::with_now_ms(1_791_600_000_000, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = board();
        fixtures::populate(&mut app);
        let target = app.workspace.selected.clone();
        press(&mut app, KeyCode::Char('1'));
        paint(&mut app, 100, 36);
        assert!(!app.usage_chart.borrow().focused());
        press(&mut app, KeyCode::Char('u'));
        assert!(app.usage_chart.borrow().focused());
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.usage_chart.borrow().selection.each_minutes, 15);
        paint(&mut app, 100, 36);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.usage_chart.borrow().selection.range_minutes, 30);
        paint(&mut app, 100, 36);
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('3'));
        let shown = text(&paint(&mut app, 100, 36));
        assert!(shown.contains("[15m]") && shown.contains("[30m]"));
        assert_eq!(app.workspace.selected, target);
        assert!(app.input.is_none());
        let range = app
            .usage_chart
            .borrow()
            .hits
            .iter()
            .find(|hit| hit.target == Target::Range(15))
            .unwrap()
            .area;
        click(&mut app, range);
        assert_eq!(app.usage_chart.borrow().selection.range_minutes, 15);
        paint(&mut app, 100, 36);
        let invalid = app
            .usage_chart
            .borrow()
            .hits
            .iter()
            .find(|hit| hit.target == Target::Each(30))
            .unwrap()
            .area;
        click(&mut app, invalid);
        assert_eq!(
            app.usage_chart.borrow().selection,
            Selection {
                each_minutes: 15,
                range_minutes: 15
            }
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("Interval is longer than the range")
        );
        assert_eq!(app.workspace.selected, target);
    });
}

#[test]
fn bucket_click_keyboard_and_hover_share_exact_unknown_evidence_with_no_close_replay() {
    crate::status::with_now_ms(1_791_600_000_000, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = board();
        fixtures::populate(&mut app);
        press(&mut app, KeyCode::Char('1'));
        paint(&mut app, 160, 36);
        let hit = app
            .usage_chart
            .borrow()
            .hits
            .iter()
            .find(|hit| hit.target == Target::Bar(2))
            .unwrap()
            .area;
        let target = app.workspace.selected.clone();
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Moved,
                column: hit.x,
                row: hit.y,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        assert_eq!(app.usage_chart.borrow().hover, Some(2));
        assert!(text(&paint(&mut app, 160, 36)).contains("Observed tokens: unavailable"));
        click(&mut app, hit);
        assert!(app.usage_details.is_some());
        assert!(app.reload_blocked());
        let captured = app.usage_details.as_ref().unwrap().borrow().lines.clone();
        assert!(captured.iter().any(|line| line.contains("0/1 agents")));
        let shown = text(&paint(&mut app, 80, 18));
        assert!(shown.contains("Usage · exact interval") && shown.contains("unavailable"));
        assert!(
            !app.frame_clocks
                .borrow()
                .iter()
                .any(|clock| matches!(clock, crate::board::view::ClockSurface::UsageChart))
        );
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.usage_details.is_none());
        assert_eq!(app.workspace.selected, target);
        paint(&mut app, 160, 36);
        press(&mut app, KeyCode::Char('u'));
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.usage_details.as_ref().unwrap().borrow().lines, captured);
    });
}

#[test]
fn resize_reader_agents_and_custom_binding_remove_old_chart_authority() {
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut app = board();
        fixtures::populate(&mut app);
        press(&mut app, KeyCode::Char('1'));
        paint(&mut app, 100, 36);
        let hit = app
            .usage_chart
            .borrow()
            .hits
            .iter()
            .find(|hit| hit.target == Target::Bar(0))
            .unwrap()
            .area;
        app.invalidate_frame_geometry();
        click(&mut app, hit);
        assert!(app.usage_details.is_none());
        let narrow = text(&paint(&mut app, 60, 8));
        assert!(
            !app.usage_chart
                .borrow()
                .hits
                .iter()
                .any(|hit| matches!(hit.target, Target::Bar(_)))
        );
        assert!(narrow.contains("Overview"));
        assert!(
            app.usage_chart
                .borrow()
                .hits
                .iter()
                .all(|hit| hit.target == Target::Controls)
        );
        press(&mut app, KeyCode::Char('2'));
        assert!(
            !app.workspace_bindings()
                .values()
                .any(|action| action.verb == Verb::Ui(OpsUiAction::UsageControls))
        );
        press(&mut app, KeyCode::Char('3'));
        app.focus = 1;
        assert!(
            !app.workspace_bindings()
                .values()
                .any(|action| action.verb == Verb::Ui(OpsUiAction::UsageControls))
        );
        app.focus = 0;
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .keys
            .get_mut(&crate::config::OpsKeyScope::Member)
            .unwrap()
            .insert(
                "u".into(),
                crate::action::Action::parse("copy {name}").unwrap(),
            );
        assert_eq!(app.workspace_bindings()["u"].verb, Verb::Copy);
        assert!(!app.usage_chart.borrow().focused());
        assert_eq!(app.selected_row().unwrap()["id"], MEMBER);
    });
}

mod evidence;
mod fixtures;
mod performance;

#[test]
fn populated_chart_cache_reuses_exact_semantics_and_same_slot_updates_revoke_it() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let mut app = board();
        fixtures::populate(&mut app);
        paint(&mut app, 100, 36);
        let before = app.usage_chart.borrow().observed.clone().unwrap();
        assert_eq!(before.buckets.len(), 12);
        assert_eq!(before.buckets[0].tokens, Some(0));
        assert_eq!(before.buckets[1].tokens, Some(1));
        assert_eq!(before.buckets[2].tokens, None);
        assert_eq!(before.buckets[3].tokens, Some(51));
        assert!(!before.buckets[3].complete());
        assert_eq!(before.buckets[10].tokens, Some(180));
        paint(&mut app, 80, 36);
        assert!(Rc::ptr_eq(
            &before,
            app.usage_chart.borrow().observed.as_ref().unwrap()
        ));
        // A real acquired replacement in the very same five-second slot must
        // invalidate even when its timestamp/grid does not change.
        fixtures::populate(&mut app);
        assert!(app.usage_chart.borrow().observed.is_none());
        paint(&mut app, 100, 36);
        assert!(!Rc::ptr_eq(
            &before,
            app.usage_chart.borrow().observed.as_ref().unwrap()
        ));
        let after = app.usage_chart.borrow().observed.clone().unwrap();
        assert_eq!(before.as_ref(), after.as_ref());
    });
}

#[test]
fn chart_shortcuts_do_not_escape_search_composers_or_modal_input() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let mut app = board();
        fixtures::populate(&mut app);
        press(&mut app, KeyCode::Char('1'));
        paint(&mut app, 100, 36);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.search, "u");
        assert!(!app.usage_chart.borrow().focused());
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('t'));
        assert!(app.input.is_some());
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "u");
        assert!(!app.usage_chart.borrow().focused());
        press(&mut app, KeyCode::Esc);
        assert!(app.input.is_none());
        press(&mut app, KeyCode::Esc); // Return retained Reader focus to the list.
        press(&mut app, KeyCode::Char('1'));
        paint(&mut app, 100, 36);
        press(&mut app, KeyCode::Char('u'));
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('u'));
        assert!(app.usage_details.is_some());
        assert_eq!(app.usage_chart.borrow().selection, Selection::default());
    });
}

#[test]
fn unavailable_idle_chart_does_not_wake_until_visible_time_evidence_changes() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let mut app = board();
        app.shell.tab = OpsTab::Overview;
        paint(&mut app, 100, 36);
        assert_eq!(
            clock_mark(&app, fixtures::NOW),
            clock_mark(&app, fixtures::NOW + 5_000)
        );
        fixtures::populate(&mut app);
        paint(&mut app, 100, 36);
        assert_ne!(
            clock_mark(&app, fixtures::NOW),
            clock_mark(&app, fixtures::NOW + 5_000)
        );
        app.help = true;
        assert_eq!(clock_mark(&app, fixtures::NOW), "");
    });
}

#[test]
fn short_terminals_collapse_to_summary_and_open_the_same_controls_without_close_replay() {
    crate::status::with_now_ms(fixtures::NOW, || {
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            let mut app = board();
            fixtures::populate(&mut app);
            app.shell.tab = tab;
            let selected = app.workspace.selected.clone();
            let shown = text(&paint(&mut app, 60, 18));
            assert!(shown.contains("TOK / 5m · observed 689 tok · partial"));
            assert!(
                app.usage_chart
                    .borrow()
                    .hits
                    .iter()
                    .all(|hit| hit.target == Target::Controls)
            );
            press(&mut app, KeyCode::Char('u'));
            assert!(app.usage_controls);
            let shown = text(&paint(&mut app, 60, 18));
            assert!(shown.contains("Usage · Each / Range"));
            assert!(shown.contains("Each") && shown.contains("Range"));
            assert_eq!(app.workspace.selected, selected);
            press(&mut app, KeyCode::Right);
            press(&mut app, KeyCode::Enter);
            assert_eq!(app.usage_chart.borrow().selection.each_minutes, 15);
            assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
            assert!(!app.usage_controls);
            assert_eq!(app.workspace.selected, selected);
            paint(&mut app, 60, 18);
            let summary = app
                .usage_chart
                .borrow()
                .hits
                .iter()
                .find(|hit| hit.target == Target::Controls)
                .unwrap()
                .area;
            click(&mut app, summary);
            assert!(app.usage_controls);
        }
    });
}

#[test]
fn overview_panels_preserve_the_wide_reader_and_talk_placement() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = board();
        fixtures::populate(&mut app);
        let target = app.workspace.selected.clone();
        paint(&mut app, 160, 24);
        let pane = app
            .conversation
            .borrow()
            .pane
            .expect("Overview reserves the wide conversation beside its panels");
        assert_eq!(pane.width, 63);
        assert!(!app.collapsed_panes().contains(&Pane::Replies));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focused(), Pane::Replies);
        press(&mut app, KeyCode::Char('t'));
        assert!(app.input.is_some());
        assert_eq!(app.shell.tab, OpsTab::Overview);
        assert_eq!(app.workspace.selected, target);
    });
}

#[test]
fn shortest_digest_body_keeps_its_last_member_row_instead_of_a_chart_summary() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let mut app = board();
        fixtures::populate(&mut app);
        app.shell.tab = OpsTab::Digest;
        paint(&mut app, 60, 8);
        assert!(!app.usage_chart.borrow().visible);
        assert!(app.hits.borrow().iter().any(|hit| hit.row().is_some()));
    });
}

#[test]
fn no_positive_reading_uses_one_summary_without_false_bar_authority() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for known_zero in [false, true] {
            let mut app = board();
            app.shell.tab = OpsTab::Overview;
            if known_zero {
                fixtures::populate_values(&mut app, [0; 12]);
            }
            let selected = app.workspace.selected.clone();
            let shown = text(&paint(&mut app, 160, 44));
            let state = app.usage_chart.borrow();
            let data = state.observed.as_ref().unwrap();
            assert!(
                data.buckets
                    .iter()
                    .all(|bucket| bucket.tokens.is_none_or(|n| n == 0))
            );
            assert_eq!(
                data.buckets.iter().any(|bucket| bucket.tokens == Some(0)),
                known_zero
            );
            assert!(!state.hits.is_empty());
            assert!(
                state.hits.iter().all(|hit| hit.target == Target::Controls),
                "no-positive data must not expose bars: {shown}"
            );
            assert!(shown.contains("TOK / 5m"));
            assert!(shown.contains(if known_zero {
                "observed 0 tok"
            } else {
                "unavailable"
            }));
            drop(state);
            press(&mut app, KeyCode::Char('u'));
            assert!(
                app.usage_controls,
                "the same Each/Range controls remain reachable"
            );
            let controls = text(&paint(&mut app, 160, 44));
            assert!(controls.contains("Usage · Each / Range"));
            assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
            assert!(!app.usage_controls);
            assert_eq!(app.workspace.selected, selected);
        }
    });
}

#[test]
fn missing_snapshot_refuses_keyboard_action_and_retained_mouse_controls() {
    crate::status::with_now_ms(fixtures::NOW, || {
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            let mut app = board();
            fixtures::populate(&mut app);
            app.shell.tab = tab;
            paint(&mut app, 60, 18);
            let summary = app
                .usage_chart
                .borrow()
                .hits
                .iter()
                .find(|hit| hit.target == Target::Controls)
                .unwrap()
                .area;
            let selected = app.workspace.selected.clone();
            // Even a retained painted hit cannot outlive its admitted source.
            app.view = None;
            assert!(!app.usage_chart_available());
            assert!(
                !app.workspace_bindings()
                    .values()
                    .any(|action| action.verb == Verb::Ui(OpsUiAction::UsageControls))
            );
            press(&mut app, KeyCode::Char('u'));
            assert!(!app.usage_controls);
            super::focus(&mut app);
            assert!(!app.usage_controls);
            click(&mut app, summary);
            assert!(!app.usage_controls);
            assert!(app.usage_details.is_none());
            assert!(!app.usage_chart.borrow().focused());
            assert_eq!(app.workspace.selected, selected);
        }
    });
}
