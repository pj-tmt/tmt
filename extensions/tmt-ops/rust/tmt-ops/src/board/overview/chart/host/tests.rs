use super::*;
use crate::action::{OpsUiAction, Verb};

#[test]
fn invalid_control_change_preserves_grid_and_inspected_bucket() {
    let mut state = State::default();
    state.apply(Target::Range(43_200)).unwrap();
    state.apply(Target::Each(Each::Fixed(60))).unwrap();
    state.bar = 23;
    let before = state.selection;
    assert!(state.apply(Target::Each(Each::Fixed(30))).is_err());
    assert_eq!(state.selection, before);
    assert_eq!(state.each, Each::Fixed(60));
    assert_eq!(state.bar, 23);
    state.apply(Target::Each(Each::Fixed(360))).unwrap();
    assert_eq!(
        state.bar, 119,
        "a newly selected grid opens at its newest bucket"
    );
}

#[test]
fn range_changes_keep_each_range_and_bucket_cursors_on_their_current_choice() {
    let app = crate::board::app::workspace::tests::board();
    for range_minutes in Selection::RANGE {
        for &each_minutes in Selection::each_choices(range_minutes) {
            let selection = Selection {
                each_minutes,
                range_minutes,
            };
            if selection.validate().is_err() {
                continue;
            }
            for next in Selection::RANGE {
                for focus in [Focus::Each, Focus::Range, Focus::Bars] {
                    let mut state = State {
                        selection,
                        bar: selection.bars().unwrap() - 1,
                        each: Each::Fixed(each_minutes),
                        ..Default::default()
                    };
                    state.set_focus(focus);
                    state.apply(Target::Range(next)).unwrap();
                    assert_eq!(state.selection, selection.with_range(next).unwrap());
                    assert_eq!(state.bar, state.selection.bars().unwrap() - 1);
                    assert_eq!(state.focus, Some(focus));
                    if focus == Focus::Bars {
                        assert_eq!(state.draft, state.bar);
                    }
                    let area = Rect::new(0, 0, 100, 1);
                    let hits = controls::header(
                        &mut Buffer::empty(area),
                        area,
                        state.selection,
                        "Usage",
                        &app,
                        state.focus,
                        state.each,
                    );
                    assert_eq!(hits.len(), 2);
                    assert_eq!(hits[0].target, Target::OpenRange);
                    assert_eq!(hits[1].target, Target::OpenEach);
                }
            }
        }
    }
}

#[test]
fn source_and_geometry_invalidation_have_distinct_lifetimes() {
    let mut state = State::default();
    state.set_focus(Focus::Range);
    state.visible = true;
    state.hits.push(Hit {
        area: Rect::new(1, 2, 3, 1),
        target: Target::Range(360),
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
fn every_range_control_fits_with_complete_padding_and_exact_hit_bounds() {
    let app = board();
    for range_minutes in Selection::RANGE {
        for &each_minutes in Selection::each_choices(range_minutes) {
            let selection = Selection {
                each_minutes,
                range_minutes,
            };
            if selection.validate().is_err() {
                continue;
            }
            for width in [37, 53, 54, 55, 60, 63, 80, 103, 148] {
                let area = Rect::new(1, 1, width, 1);
                let mut buffer = Buffer::empty(Rect::new(0, 0, width + 2, 3));
                let hits = controls::header(
                    &mut buffer,
                    area,
                    selection,
                    "Usage",
                    &app,
                    None,
                    Each::Fixed(each_minutes),
                );
                assert_eq!(hits.len(), 2);
                for hit in hits {
                    assert_eq!(hit.area.intersection(area), hit.area);
                    let label = match hit.target {
                        Target::OpenEach => {
                            format!(" Each {} ▾ ", super::super::format::interval(each_minutes))
                        }
                        Target::OpenRange => {
                            format!(" Range {} ▾ ", super::super::format::range(range_minutes))
                        }
                        _ => unreachable!(),
                    };
                    let shown: String = (hit.area.x..hit.area.right())
                        .map(|x| buffer[(x, hit.area.y)].symbol())
                        .collect();
                    assert_eq!(shown, label, "{selection:?}/{width}");
                }
            }
        }
    }
}

#[test]
fn header_keeps_caption_and_independent_dropdowns_in_their_tracks() {
    let app = board();
    for range_minutes in Selection::RANGE {
        let selection = Each::Auto.resolve(range_minutes, 40);
        let title = format!(
            "Usage · last {} · ~123.4k tok",
            super::super::format::range(range_minutes)
        );
        for width in [60, 79, 80, 81, 84, 85, 103, 148] {
            let area = Rect::new(0, 0, width, 1);
            let mut buffer = Buffer::empty(area);
            let hits =
                controls::header(&mut buffer, area, selection, &title, &app, None, Each::Auto);
            assert!(hits.iter().all(|h| h.area.intersection(area) == h.area));
            if hits.len() == 1 {
                assert_eq!(hits[0].target, Target::Controls);
                assert!(text(&buffer).contains(&title));
                continue;
            }
            assert_eq!(hits.len(), 2);
            assert_eq!(hits[1].area.right(), area.right() - 1);
            let shown = text(&buffer);
            assert!(shown.contains("Each Auto ("));
            if width >= 103 {
                assert!(shown.contains(&title), "{shown}");
            }
        }
    }
}

#[test]
fn long_ranges_render_exact_buckets_and_reject_old_or_inactive_demand() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for range_minutes in Selection::RANGE.into_iter().skip(1) {
            for &each_minutes in Selection::each_choices(range_minutes) {
                let mut app = board();
                fixtures::mixed(&mut app);
                let selection = Selection {
                    each_minutes,
                    range_minutes,
                };
                if selection.validate().is_err() {
                    continue;
                }
                app.usage_chart.borrow_mut().each = Each::Fixed(each_minutes);
                app.usage_chart
                    .borrow_mut()
                    .selection
                    .change(selection)
                    .unwrap();
                let request = app.retained_history_request().unwrap();
                let stale = fixtures::retained(
                    &app,
                    selection
                        .with_range(if range_minutes == 360 { 1_440 } else { 360 })
                        .unwrap(),
                );
                assert!(!app.apply_history(stale, std::time::Instant::now()));
                let read = fixtures::retained(&app, selection);
                assert!(app.apply_history(read, std::time::Instant::now()));
                for width in [40, 53, 54, 55, 60, 103, 148] {
                    let shown = text(&paint(&mut app, width, 51));
                    let state = app.usage_chart.borrow();
                    let observed = state.observed.as_ref().unwrap();
                    assert_eq!(observed.buckets.len(), request.buckets());
                    assert_eq!(observed.buckets[0].tokens, None);
                    if observed.buckets.len() > 2 {
                        assert!(!observed.buckets[1].complete());
                        assert_eq!(observed.buckets[2].tokens, Some(153));
                    }
                    if width >= 103 {
                        assert!(shown.contains(&format!(
                            "Usage · last {}",
                            super::super::format::range(range_minutes)
                        )));
                        if observed.buckets.iter().any(|b| b.tokens.is_some()) {
                            assert!(
                                state
                                    .hits
                                    .iter()
                                    .any(|hit| hit.target == Target::Bar(request.buckets() - 1))
                            );
                        } else {
                            assert!(
                                !state
                                    .hits
                                    .iter()
                                    .any(|hit| matches!(hit.target, Target::Bar(_)))
                            );
                        }
                    }
                }
                assert_eq!(
                    clock_mark(&app, fixtures::NOW),
                    clock_mark(&app, fixtures::NOW + 5_000)
                );
                if app
                    .usage_chart
                    .borrow()
                    .observed
                    .as_ref()
                    .unwrap()
                    .buckets
                    .iter()
                    .any(|b| b.tokens.is_some())
                {
                    assert_ne!(
                        clock_mark(&app, fixtures::NOW),
                        clock_mark(&app, fixtures::NOW + 300_000)
                    );
                } else {
                    assert_eq!(
                        clock_mark(&app, fixtures::NOW + 300_000),
                        "",
                        "unavailable chart adds no clock"
                    );
                }
                app.shell.tab = OpsTab::Agents;
                assert!(app.retained_history_request().is_none());
                let read = fixtures::retained(&app, selection);
                assert!(!app.apply_history(read, std::time::Instant::now()));
                app.shell.tab = OpsTab::Digest;
                assert_eq!(app.retained_history_request(), Some(request));
                paint(&mut app, 148, 51);
            }
        }
    });
}

#[test]
fn current_controls_and_keyboard_share_grid_without_recipient_or_draft_changes() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = board();
        fixtures::populate(&mut app);
        press(&mut app, KeyCode::Char('1'));
        paint(&mut app, 103, 51);
        let target = app.workspace.selected.clone();
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.usage_chart.borrow().focus, Some(Focus::Range));
        press(&mut app, KeyCode::Enter);
        assert!(app.usage_controls);
        paint(&mut app, 103, 51);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert!(!app.usage_controls);
        paint(&mut app, 103, 51);
        assert_eq!(app.usage_chart.borrow().selection.range_minutes, 360);
        let each = app
            .usage_chart
            .borrow()
            .hits
            .iter()
            .find(|h| h.target == Target::OpenEach)
            .unwrap()
            .area;
        click(&mut app, each);
        assert!(app.usage_controls);
        paint(&mut app, 103, 51);
        press(&mut app, KeyCode::Down); // Auto -> 5m
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.usage_chart.borrow().each, Each::Fixed(5));
        assert_eq!(app.usage_chart.borrow().selection.bars(), Ok(72));
        assert_eq!(app.workspace.selected, target);
        assert!(app.input.is_none());
        app.invalidate_frame_geometry();
        click(&mut app, each);
        assert!(
            !app.usage_controls,
            "stale geometry cannot reopen a dropdown"
        );
    });
}

#[test]
fn bucket_click_keyboard_and_hover_share_exact_unknown_evidence_with_no_close_replay() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = board();
        fixtures::populate(&mut app);
        press(&mut app, KeyCode::Char('1'));
        paint(&mut app, 148, 51);
        let hit = app
            .usage_chart
            .borrow()
            .hits
            .iter()
            .find(|h| h.target == Target::Bar(2))
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
        let expected =
            project::readout(&app.usage_chart.borrow().observed.as_ref().unwrap().buckets[2]);
        assert!(expected.ends_with("— tok"));
        assert!(text(&paint(&mut app, 148, 51)).contains(&expected));
        click(&mut app, hit);
        let pinned = app.usage_chart.borrow().pinned;
        assert!(pinned.is_some());
        assert!(
            !app.modal_open(),
            "readout pins inline and opens no Details window"
        );
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Moved,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        assert!(text(&paint(&mut app, 148, 51)).contains(&expected));
        assert_eq!(app.usage_chart.borrow().pinned, pinned);
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.usage_chart.borrow().pinned.is_none());
        assert_eq!(app.workspace.selected, target);
        assert!(text(&paint(&mut app, 148, 51)).contains("Usage · last 1h"));
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
        assert!(app.usage_chart.borrow().pinned.is_none());
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

pub(in crate::board) mod cases;
mod evidence;
pub(in crate::board) mod fixtures;
mod navigation;
mod performance;

#[test]
fn populated_chart_cache_reuses_exact_semantics_and_same_slot_updates_revoke_it() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let mut app = board();
        fixtures::populate(&mut app);
        app.usage_chart
            .borrow_mut()
            .apply(Target::Each(Each::Fixed(5)))
            .unwrap();
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
        paint(&mut app, 100, 36);
        assert_eq!(app.search, "u");
        assert!(!app.usage_chart.borrow().focused());
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_some());
        press(&mut app, KeyCode::Char('u'));
        paint(&mut app, 100, 36);
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "u");
        assert!(!app.usage_chart.borrow().focused());
        press(&mut app, KeyCode::Esc);
        assert!(app.input.is_none());
        press(&mut app, KeyCode::Esc); // Return retained Reader focus to the list.
        press(&mut app, KeyCode::Char('1'));
        paint(&mut app, 100, 36);
        press(&mut app, KeyCode::Char('u'));
        paint(&mut app, 100, 36);
        press(&mut app, KeyCode::Enter); // Range dropdown owns input.
        press(&mut app, KeyCode::Char('u'));
        paint(&mut app, 100, 36);
        assert!(app.usage_controls);
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
            app.usage_chart
                .borrow_mut()
                .apply(Target::Each(Each::Fixed(5)))
                .unwrap();
            let selected = app.workspace.selected.clone();
            let shown = text(&paint(&mut app, 60, 14));
            assert!(
                shown.contains("Usage · last 1h · ~689 tok"),
                "{tab:?}: {shown}"
            );
            assert!(
                app.usage_chart.borrow().hits.iter().all(|hit| matches!(
                    hit.target,
                    Target::Controls | Target::OpenEach | Target::OpenRange
                )),
                "summary has only complete control triggers, no bar hits"
            );
            press(&mut app, KeyCode::Char('u'));
            assert!(!app.usage_controls);
            press(&mut app, KeyCode::Enter);
            assert!(app.usage_controls);
            let shown = text(&paint(&mut app, 60, 14));
            assert!(shown.contains("Range"), "{shown}");
            press(&mut app, KeyCode::Down);
            press(&mut app, KeyCode::Enter);
            assert!(!app.usage_controls);
            assert_eq!(app.usage_chart.borrow().selection.range_minutes, 360);
            assert_eq!(app.workspace.selected, selected);
            assert!(app.input.is_none());
            paint(&mut app, 60, 14);
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.workspace.selected, selected);
        }
    });
}

#[test]
fn overview_panels_preserve_the_wide_reader_and_explicit_talk_placement() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = board();
        crate::board::app::workspace::tests::observe_waiting_member(&mut app);
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
        // Talk remains an explicit action; factory Enter now answers a request.
        app.perform(&crate::action::Action::parse("talk").unwrap());
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
                state.hits.iter().all(|hit| matches!(
                    hit.target,
                    Target::Controls | Target::OpenEach | Target::OpenRange | Target::Quota(_)
                ) || known_zero
                    && matches!(hit.target, Target::Bar(_))),
                "missing data exposes no bars; known zero remains inspectable: {shown}"
            );
            assert!(shown.contains("Usage · last 1h") && shown.contains("Each Auto (5m)"));
            let expected = if known_zero {
                format!(
                    "Usage · last 1h · {}0 tok",
                    if data.buckets.iter().all(|bucket| bucket.complete()) {
                        ""
                    } else {
                        "~"
                    }
                )
            } else {
                "Usage · last 1h · — tok".into()
            };
            assert!(shown.contains(&expected), "{shown}");
            drop(state);
            press(&mut app, KeyCode::Char('u'));
            paint(&mut app, 100, 36);
            press(&mut app, KeyCode::Enter);
            assert!(
                app.usage_controls,
                "Range dropdown remains reachable without positive tokens"
            );
            let controls = text(&paint(&mut app, 160, 44));
            assert!(controls.contains("Range") && controls.contains("24h"));
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
            paint(&mut app, 60, 14);
            let summary = app
                .usage_chart
                .borrow()
                .hits
                .iter()
                .find(|hit| matches!(hit.target, Target::Controls | Target::OpenRange))
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
            paint(&mut app, 100, 36);
            assert!(!app.usage_controls);
            super::focus(&mut app);
            assert!(!app.usage_controls);
            click(&mut app, summary);
            assert!(!app.usage_controls);
            assert!(app.usage_chart.borrow().pinned.is_none());
            assert!(!app.usage_chart.borrow().focused());
            assert_eq!(app.workspace.selected, selected);
        }
    });
}

#[test]
fn demand_bounds_cells_hits_and_remaining_target_budget() {
    crate::status::with_now_ms(fixtures::NOW, || {
        for populated in [false, true] {
            for tab in [OpsTab::Overview, OpsTab::Digest] {
                for width in [0, 37, 38, 53, 54, 60, 100, 160] {
                    for height in 0..=44 {
                        let mut app = board();
                        if populated {
                            fixtures::populate(&mut app);
                        }
                        app.shell.tab = tab;
                        let used = demand(&app, width, height, fixtures::NOW);
                        assert!(used <= height);
                        let rect = Rect::new(2, 2, width, used);
                        let mut terminal = Terminal::new(TestBackend::new(164, 48)).unwrap();
                        terminal
                            .draw(|frame| render_panel(frame, &app, rect, fixtures::NOW))
                            .unwrap();
                        assert!(
                            app.usage_chart
                                .borrow()
                                .hits
                                .iter()
                                .all(|hit| hit.area.intersection(rect) == hit.area)
                        );
                        let buffer = terminal.backend().buffer();
                        for y in 0..48 {
                            for x in 0..164 {
                                if !rect.contains((x, y).into()) {
                                    assert_eq!(buffer[(x, y)].symbol(), " ");
                                }
                            }
                        }
                        if used > 1 {
                            assert!(height - used >= 6, "{tab:?}/{width}/{height}");
                        }
                    }
                }
            }
        }
    });
}

#[test]
fn chart_budget_and_painter_share_the_captured_existing_clock_across_a_rollover() {
    let mut app = board();
    app.shell.tab = OpsTab::Overview;
    let area = Rect::new(0, 0, 100, 36);
    let mut original = Terminal::new(TestBackend::new(100, 36)).unwrap();
    let used = crate::status::with_now_ms(fixtures::NOW, || {
        fixtures::populate(&mut app);
        let used = demand(&app, area.width, area.height, fixtures::NOW);
        original
            .draw(|frame| {
                render_panel(
                    frame,
                    &app,
                    Rect {
                        height: used,
                        ..area
                    },
                    fixtures::NOW,
                );
            })
            .unwrap();
        used
    });
    let mut rolled = Terminal::new(TestBackend::new(100, 36)).unwrap();
    crate::status::with_now_ms(fixtures::NOW + 3_600_000, || {
        rolled
            .draw(|frame| {
                render_panel(
                    frame,
                    &app,
                    Rect {
                        height: used,
                        ..area
                    },
                    fixtures::NOW,
                )
            })
            .unwrap();
    });
    assert_eq!(rolled.backend().buffer(), original.backend().buffer());
}

#[test]
#[ignore = "writes task-owned current Overview evidence; never frozen fixtures"]
fn capture_overview_usage_packet() {
    use tmt_cli_style::{Depth, Theme, theme::Base};
    if let Ok(path) = std::env::var("TMT_QUOTA_CAPTURE") {
        capture_quota_packet(&path);
        return;
    }
    if let Ok(path) = std::env::var("TMT_USAGE_LONG_CAPTURE") {
        capture_long_usage_packet(&path);
        return;
    }
    let path = std::env::var("TMT_OVERVIEW_CAPTURE").expect("task-owned evidence path");
    let mut captures = Vec::new();
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for width in [160, 148, 103, 100, 60] {
            for height in [51, 36, 18, 8] {
                for (base, depth) in [
                    (Base::Tmt, Depth::TrueColor),
                    (Base::TmtLight, Depth::TrueColor),
                    (Base::Tmt, Depth::None),
                ] {
                    for no_data in [false, true] {
                        let mut app = board();
                        if no_data {
                            fixtures::unreported(&mut app);
                        } else {
                            fixtures::mixed(&mut app);
                        }
                        app.view.as_mut().unwrap().look = crate::look::Look {
                            theme: Theme::new(base),
                            depth,
                        };
                        app.local_zone = Some(jiff::tz::TimeZone::UTC);
                        let buffer = paint(&mut app, width, height);
                        let cells=buffer.content.iter().map(|cell|serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                        let lines = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                            .collect::<Vec<_>>();
                        captures.push(serde_json::json!({"scene":if no_data {"overview-usage-empty"}else{"overview-usage"},"width":width,"height":height,"base":base.name(),"depth":format!("{depth:?}"),"cells":cells,"lines":lines,"selected":format!("{:?}",app.workspace.selected),"chartHits":app.usage_chart.borrow().hits.len()}));
                    }
                }
            }
        }
    });
    assert_eq!(captures.len(), 120);
    std::fs::write(path, serde_json::to_vec(&captures).unwrap()).unwrap();
}

fn capture_long_usage_packet(path: &str) {
    use tmt_cli_style::{Depth, Theme, theme::Base};
    let mut captures = Vec::new();
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for range in Selection::RANGE {
            for width in [103, 148] {
                for (base, depth) in [
                    (Base::Tmt, Depth::TrueColor),
                    (Base::TmtLight, Depth::TrueColor),
                    (Base::Tmt, Depth::None),
                ] {
                    for empty in [false, true] {
                        let mut app = board();
                        if empty {
                            fixtures::unreported(&mut app);
                        } else {
                            fixtures::mixed(&mut app);
                        }
                        app.usage_chart
                            .borrow_mut()
                            .apply(Target::Range(range))
                            .unwrap();
                        let selection = app.usage_chart.borrow().selection;
                        if range > 60 && !empty {
                            let read = fixtures::retained(&app, selection);
                            assert!(app.apply_history(read, std::time::Instant::now()));
                        }
                        app.view.as_mut().unwrap().look = crate::look::Look {
                            theme: Theme::new(base),
                            depth,
                        };
                        app.local_zone = Some(jiff::tz::TimeZone::UTC);
                        let buffer = paint(&mut app, width, 51);
                        let cells: Vec<_> = buffer.content.iter().map(|cell| serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect();
                        let lines: Vec<_> = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                            .collect();
                        captures.push(serde_json::json!({"scene":if empty {"long-empty"}else{"long-partial"},"range":range,"each":selection.each_minutes,"width":width,"height":51,"base":base.name(),"depth":format!("{depth:?}"),"cells":cells,"lines":lines,"chartHits":app.usage_chart.borrow().hits.len()}));
                    }
                }
            }
        }
    });
    assert_eq!(captures.len(), 60);
    std::fs::write(path, serde_json::to_vec(&captures).unwrap()).unwrap();
}

#[test]
fn quota_chips_are_shared_keyboard_reachable_and_preserve_token_demand_and_pane_tab() {
    use crate::board::limits::QuotaWindow;
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for width in [103, 148] {
            let mut app = board();
            crate::board::app::workspace::tests::observe_waiting_member(&mut app);
            fixtures::populate(&mut app);
            let before = app.history_demand();
            paint(&mut app, width, 51);
            let source = Rc::clone(app.usage_chart.borrow().observed.as_ref().unwrap());
            let short = app
                .usage_chart
                .borrow()
                .hits
                .iter()
                .find(|hit| hit.target == Target::Quota(QuotaWindow::Short))
                .unwrap()
                .area;
            click(&mut app, short);
            assert_eq!(app.usage_chart.borrow().quota_window, QuotaWindow::Short);
            assert_eq!(app.history_demand(), before);
            assert!(Rc::ptr_eq(
                &source,
                app.usage_chart.borrow().observed.as_ref().unwrap()
            ));
            assert!(app.usage_chart.borrow().hits.is_empty());
            paint(&mut app, width, 51);
            press(&mut app, KeyCode::Char('u'));
            paint(&mut app, width, 51);
            press(&mut app, KeyCode::Tab);
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.usage_chart.borrow().focus, Some(Focus::Quota));
            press(&mut app, KeyCode::Left);
            press(&mut app, KeyCode::Enter);
            assert_eq!(app.usage_chart.borrow().quota_window, QuotaWindow::Weekly);
            press(&mut app, KeyCode::Esc);
            paint(&mut app, width, 51);
            assert!(!app.usage_chart.borrow().focused());
            paint(&mut app, 160, 51); // Admit the waiting agent conversation for the pane positive control.
            let focus = app.focus;
            press(&mut app, KeyCode::Tab);
            assert_ne!(
                app.focus, focus,
                "Tab retains pane ownership after leaving controls"
            );
        }
    });
}

fn capture_quota_packet(path: &str) {
    use crate::board::limits::QuotaWindow;
    use tmt_cli_style::{Depth, Theme, theme::Base};
    let mut captures = Vec::new();
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for scenario in ["rising", "falling", "steady", "unknown", "early"] {
            for width in [103, 148] {
                for (base, depth) in [
                    (Base::Tmt, Depth::TrueColor),
                    (Base::TmtLight, Depth::TrueColor),
                    (Base::Tmt, Depth::None),
                ] {
                    for kind in QuotaWindow::ALL {
                        let mut app = board();
                        fixtures::quota(&mut app, scenario);
                        app.usage_chart.borrow_mut().restore_quota_window(kind);
                        app.view.as_mut().unwrap().look = crate::look::Look {
                            theme: Theme::new(base),
                            depth,
                        };
                        app.local_zone = Some(jiff::tz::TimeZone::UTC);
                        let buffer = paint(&mut app, width, 51);
                        let cells=buffer.content.iter().map(|cell|serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                        let lines = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                            .collect::<Vec<_>>();
                        captures.push(serde_json::json!({"scene":scenario,"width":width,"height":51,"base":base.name(),"depth":format!("{depth:?}"),"quotaWindow":kind.key(),"cells":cells,"lines":lines}));
                    }
                }
            }
        }
    });
    assert_eq!(captures.len(), 60);
    std::fs::write(path, serde_json::to_vec(&captures).unwrap()).unwrap();
}

#[test]
fn quota_bucket_arithmetic_and_actual_frames_are_independent_of_every_token_selection() {
    use crate::board::limits::{QuotaWindow, Trend};
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for (scenario, expected, trend) in [
            ("rising", Some(0.6), Some(Trend::Up)),
            ("falling", Some(0.3), Some(Trend::Down)),
            ("steady", Some(0.6), Some(Trend::Same)),
            ("early", Some(6.0), Some(Trend::Up)),
            ("unknown", None, None),
        ] {
            let mut app = board();
            fixtures::quota(&mut app, scenario);
            for kind in QuotaWindow::ALL {
                let before =
                    crate::board::overview::quota::projection(&app, "codex", kind, fixtures::NOW);
                match (before.pace.per_hour, expected) {
                    (Some(rate), Some(expected)) => assert!(
                        (rate - expected).abs() < 1e-10,
                        "{scenario}/{kind:?}: {rate}"
                    ),
                    (None, None) => (),
                    other => panic!("{scenario}/{kind:?}: {other:?}"),
                };
                assert_eq!(before.pace.trend, trend, "{scenario}/{kind:?}");
                for range in Selection::RANGE {
                    app.usage_chart
                        .borrow_mut()
                        .apply(Target::Range(range))
                        .unwrap();
                    for &each in Selection::each_choices(range) {
                        if (Selection {
                            each_minutes: each,
                            range_minutes: range,
                        })
                        .validate()
                        .is_err()
                        {
                            continue;
                        }
                        app.usage_chart
                            .borrow_mut()
                            .apply(Target::Each(Each::Fixed(each)))
                            .unwrap();
                        let after = crate::board::overview::quota::projection(
                            &app,
                            "codex",
                            kind,
                            fixtures::NOW,
                        );
                        assert_eq!(after.pace, before.pace);
                        assert_eq!(after.balance, before.balance);
                        assert!(app.history_demand().unwrap().quota);
                    }
                }
            }
            app.usage_chart
                .borrow_mut()
                .apply(Target::Range(60))
                .unwrap();
            app.usage_chart
                .borrow_mut()
                .apply(Target::Each(Each::Fixed(5)))
                .unwrap();
            for width in [103, 148] {
                let buffer = paint(&mut app, width, 51);
                let left = if width == 103 { 65 } else { 98 };
                let state = app.usage_chart.borrow();
                let weekly = state
                    .hits
                    .iter()
                    .find(|hit| hit.target == Target::Quota(QuotaWindow::Weekly))
                    .unwrap()
                    .area;
                let short = state
                    .hits
                    .iter()
                    .find(|hit| hit.target == Target::Quota(QuotaWindow::Short))
                    .unwrap()
                    .area;
                assert_eq!((weekly.x, short.x), (left + 6, left + 11));
                assert_eq!(buffer[(left, weekly.y)].symbol(), "q");
                assert_eq!(weekly.width, 4);
                assert_eq!(short.width, 4);
            }
        }
    });
}

#[test]
fn quota_group_admits_both_whole_choices_or_none_and_authored_navigation_wins() {
    use crate::board::limits::QuotaWindow;
    for width in [14, 15, 37] {
        let app = board();
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        let hits = controls::quota(
            &mut buffer,
            area,
            QuotaWindow::Weekly,
            Some(Focus::Quota),
            1,
            &app,
        );
        if width < 15 {
            assert!(hits.is_empty());
            assert!(buffer.content.iter().all(|cell| cell.symbol() == " "));
        } else {
            assert_eq!(hits.len(), 2);
            for hit in hits {
                assert_eq!(hit.area.intersection(area), hit.area);
                assert_eq!(hit.area.width, 4);
            }
        }
    }
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = board();
        fixtures::populate(&mut app);
        crate::board::app::tests::configure_workspace(&mut app, "[bind]\nright='menu'\n");
        paint(&mut app, 160, 51);
        app.usage_chart.borrow_mut().set_focus(Focus::Quota);
        let before = app.usage_chart.borrow().quota_window;
        press(&mut app, KeyCode::Right);
        assert!(app.menu.is_some());
        assert!(!app.usage_chart.borrow().focused());
        assert_eq!(app.usage_chart.borrow().quota_window, before);
    });
}
