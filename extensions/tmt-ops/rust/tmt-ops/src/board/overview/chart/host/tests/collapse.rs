//! Automatic placement over real acquired Rate history, not rounded bar cells.
use super::*;
use crate::board::app::Choice;
use tmt_cli_style::{Depth, Theme, theme::Base};

#[derive(Clone, Copy, Debug)]
enum Reading {
    Unknown,
    Zero,
    ZeroPartial,
    One,
    OnePartial,
}

fn acquire(app: &mut App, reading: Reading) {
    fixtures::populate_with(app, |index, slice| {
        slice.input_tokens = if index == 0 && matches!(reading, Reading::One | Reading::OnePartial)
        {
            1
        } else {
            0
        };
        slice.output_tokens = 0;
        slice.cached_input_tokens = 0;
        let unknown = matches!(reading, Reading::Unknown)
            || (matches!(reading, Reading::ZeroPartial | Reading::OnePartial)
                && (20..30).contains(&index));
        let partial = matches!(reading, Reading::ZeroPartial | Reading::OnePartial) && index < 10;
        slice.covered_ms = if unknown {
            0
        } else if partial {
            10_000
        } else {
            30_000
        };
        slice.complete = !unknown && !partial;
        slice.gap = unknown || partial;
    });
}

fn choose_menu_action(app: &mut App, verb: Verb) {
    paint(app, 100, 36);
    let index = app
        .menu
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|entry| matches!(&entry.choice, Choice::Action(action) if action.verb == verb))
        .expect("the existing menu retains the requested action");
    for _ in 0..index {
        press(app, KeyCode::Down);
    }
    press(app, KeyCode::Enter);
}

#[test]
fn no_positive_count_reclaims_rows_and_keeps_unknown_zero_and_partial_truth() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            for (reading, label) in [
                (Reading::Unknown, "unavailable"),
                (Reading::Zero, "observed 0 tok"),
                (Reading::ZeroPartial, "observed 0 tok · partial"),
            ] {
                for (base, depth) in [
                    (Base::Tmt, Depth::TrueColor),
                    (Base::TmtLight, Depth::TrueColor),
                    (Base::Tmt, Depth::None),
                ] {
                    for (width, height) in [(160, 36), (100, 36), (80, 36), (60, 18)] {
                        let mut app = board();
                        acquire(&mut app, reading);
                        app.shell.tab = tab;
                        let view = app.view.as_mut().unwrap();
                        view.look.theme = Theme::new(base);
                        view.look.depth = depth;
                        let target = app.workspace.selected.clone();
                        let shown = text(&paint(&mut app, width, height));
                        assert!(
                            shown.contains(&format!("TOK / 5m · {label} · u chart")),
                            "{tab:?}/{reading:?}/{width}x{height}: {shown}"
                        );
                        assert!(!shown.contains("tokens unknown"));
                        let state = app.usage_chart.borrow();
                        assert_eq!(state.hits.len(), 1);
                        assert_eq!(state.hits[0].target, Target::Controls);
                        assert_eq!(state.hits[0].area.height, 1);
                        assert!(state.hover.is_none() && !state.focused());
                        assert_eq!(app.workspace.selected, target);
                        let observed = state.observed.as_ref().unwrap();
                        assert!(
                            observed
                                .buckets
                                .iter()
                                .all(|b| b.tokens.is_none_or(|v| v == 0))
                        );
                        assert_eq!(
                            observed.buckets.iter().any(|b| b.tokens.is_some()),
                            !matches!(reading, Reading::Unknown),
                            "the summary must not reinterpret unknown as zero"
                        );
                    }
                }
            }
        }
    });
}

#[test]
fn one_raw_token_even_with_partial_coverage_keeps_the_chart() {
    crate::status::with_now_ms(fixtures::NOW, || {
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            for reading in [Reading::One, Reading::OnePartial] {
                for width in [160, 100, 80] {
                    let mut app = board();
                    acquire(&mut app, reading);
                    app.shell.tab = tab;
                    let shown = text(&paint(&mut app, width, 36));
                    assert!(shown.contains("TOK / 5m · unique identities"));
                    let state = app.usage_chart.borrow();
                    assert!(
                        state
                            .hits
                            .iter()
                            .any(|hit| matches!(hit.target, Target::Bar(_)))
                    );
                    let data = state.observed.as_ref().unwrap();
                    assert_eq!(
                        data.buckets.iter().filter_map(|b| b.tokens).sum::<u128>(),
                        1
                    );
                    if matches!(reading, Reading::OnePartial) {
                        assert!(
                            data.buckets
                                .iter()
                                .any(|b| b.tokens == Some(1) && !b.complete())
                        );
                    }
                }
            }
        }
    });
}

#[test]
fn collapsed_summary_key_pointer_and_actions_keep_controls_and_exact_inspection() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            for pointer in ["key", "summary", "actions"] {
                let mut app = board();
                app.shell.tab = tab;
                let target = app.workspace.selected.clone();
                paint(&mut app, 100, 36);
                match pointer {
                    "key" => {
                        press(&mut app, KeyCode::Char('u'));
                    }
                    "summary" => {
                        let area = app.usage_chart.borrow().hits[0].area;
                        click(&mut app, area);
                    }
                    _ => {
                        press(&mut app, KeyCode::Char(','));
                        choose_menu_action(&mut app, Verb::Menu);
                        choose_menu_action(&mut app, Verb::Ui(OpsUiAction::UsageControls));
                    }
                }
                assert!(app.usage_controls);
                let shown = text(&paint(&mut app, 100, 36));
                assert!(shown.contains("Usage · Each / Range"));
                assert!(shown.contains("Each") && shown.contains("Range"));
                press(&mut app, KeyCode::Tab);
                press(&mut app, KeyCode::Tab);
                press(&mut app, KeyCode::Enter);
                assert!(app.usage_details.is_some());
                assert!(text(&paint(&mut app, 100, 36)).contains("Observed tokens: unavailable"));
                assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
                assert!(app.usage_details.is_none() && app.usage_controls);
                paint(&mut app, 100, 36);
                assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
                assert!(!app.usage_controls && app.workspace.selected == target);
                paint(&mut app, 100, 36);
                assert!(!app.usage_chart.borrow().focused());
            }
        }
    });
}

#[test]
fn an_empty_held_series_uses_unavailable_summary_without_inventing_zero() {
    crate::status::with_now_ms(fixtures::NOW, || {
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            let mut app = board();
            app.shell.tab = tab;
            let mut empty = app.usage_series(Selection::default(), fixtures::NOW);
            // Production's admitted interval grid is nonempty. Pin the
            // placement fallback explicitly, rather than relying on all().
            empty.buckets.clear();
            app.usage_chart.get_mut().observed = Some(Rc::new(empty));
            assert!(text(&paint(&mut app, 100, 36)).contains("TOK / 5m · unavailable · u chart"));
            assert!(
                app.usage_chart
                    .borrow()
                    .hits
                    .iter()
                    .all(|hit| hit.target == Target::Controls)
            );
        }
    });
}

#[test]
fn zero_counts_remain_exact_in_explicit_controls_and_partial_inspection() {
    crate::status::with_now_ms(fixtures::NOW, || {
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            for reading in [Reading::Zero, Reading::ZeroPartial] {
                let mut app = board();
                acquire(&mut app, reading);
                app.shell.tab = tab;
                paint(&mut app, 100, 36);
                press(&mut app, KeyCode::Char('u'));
                assert!(app.usage_controls);
                paint(&mut app, 100, 36);
                let expected = project::details(
                    &app.usage_chart.borrow().observed.as_ref().unwrap().buckets[0],
                );
                press(&mut app, KeyCode::Tab);
                press(&mut app, KeyCode::Tab);
                press(&mut app, KeyCode::Enter);
                let details = app.usage_details.as_ref().unwrap().borrow();
                assert_eq!(details.lines, expected);
                assert!(
                    details
                        .lines
                        .iter()
                        .any(|line| line.starts_with("Observed tokens: 0"))
                );
                assert!(
                    !details
                        .lines
                        .iter()
                        .any(|line| line == "Observed tokens: unavailable")
                );
            }
        }
    });
}

#[test]
fn source_disappearance_and_return_revoke_plot_authority_without_losing_session_state() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            let mut app = crate::board::app::workspace::tests::populated_board(8, 100);
            app.shell.tab = tab;
            let target = app
                .workspace_rows()
                .iter()
                .skip(15)
                .find(|row| {
                    matches!(
                        row.target,
                        crate::board::workspace::OpsTarget::Member { .. }
                    )
                })
                .unwrap()
                .target
                .clone();
            assert!(matches!(
                target,
                crate::board::workspace::OpsTarget::Member { .. }
            ));
            app.workspace.selected = Some(target.clone());
            app.workspace_reconcile();
            paint(&mut app, 100, 36);
            press(&mut app, KeyCode::Char('t'));
            app.paste("retained recipient draft");
            let input = app.input.as_ref().unwrap();
            let draft_key = input.draft_key.clone();
            let recipient = input.row_send.as_ref().unwrap().target.clone();
            press(&mut app, KeyCode::Esc);
            press(&mut app, KeyCode::Esc);
            fixtures::populate(&mut app);
            app.shell.tab = tab;
            app.scrolls.restore([(Pane::Rows, 10)]);
            paint(&mut app, 100, 36);
            assert_eq!(app.scrolls.offset(Pane::Rows), 10);
            let selection = app.usage_chart.borrow().selection;
            let hit = app
                .usage_chart
                .borrow()
                .hits
                .iter()
                .find(|hit| hit.target == Target::Bar(1))
                .unwrap()
                .area;
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Moved,
                    column: hit.x,
                    row: hit.y,
                    modifiers: KeyModifiers::NONE,
                },
                std::time::Instant::now(),
            );
            press(&mut app, KeyCode::Char('u'));
            assert!(app.usage_chart.borrow().focused());
            assert_eq!(app.usage_chart.borrow().hover, Some(1));

            // A worker publishes a separately acquired view; it does not take
            // the retained view out of App before applying the snapshot.
            let replace_view = |app: &mut App, mut acquired: App| {
                let mut snapshot =
                    crate::board::app::tests::snapshot(crate::board::ALL, serde_json::json!([]));
                snapshot.tabs = app.tabs.clone();
                snapshot.view = Ok(acquired.view.take().unwrap());
                app.apply(snapshot);
            };
            replace_view(
                &mut app,
                crate::board::app::workspace::tests::populated_board(8, 100),
            );
            let shown = text(&paint(&mut app, 100, 36));
            assert!(shown.contains("TOK / 5m · unavailable · u chart"));
            let state = app.usage_chart.borrow();
            assert!(!state.focused() && state.hover.is_none());
            assert!(state.hits.iter().all(|hit| hit.target == Target::Controls));
            assert_eq!(state.selection, selection);
            drop(state);
            assert_eq!(app.workspace.selected.as_ref(), Some(&target));
            assert_eq!(app.scrolls.offset(Pane::Rows), 10);
            click(&mut app, hit);
            assert!(
                app.usage_details.is_none(),
                "stale bar coordinates cannot inspect a revoked bucket"
            );
            // The same coordinates may now select a list row. Restore the
            // original UUID explicitly before checking its retained draft.
            app.workspace.selected = Some(target.clone());
            app.workspace_reconcile();
            let mut acquired = crate::board::app::workspace::tests::populated_board(8, 100);
            acquired.view = Some(fixtures::seed_view(
                acquired.view.take().unwrap(),
                |_, _| {},
            ));
            replace_view(&mut app, acquired);
            paint(&mut app, 100, 36);
            assert_eq!(app.scrolls.offset(Pane::Rows), 10);
            assert!(
                app.usage_chart
                    .borrow()
                    .hits
                    .iter()
                    .any(|hit| matches!(hit.target, Target::Bar(_)))
            );
            assert_eq!(app.usage_chart.borrow().selection, selection);
            assert_eq!(app.workspace.selected.as_ref(), Some(&target));
            press(&mut app, KeyCode::Char('t'));
            let input = app.input.as_ref().unwrap();
            assert_eq!(input.text, "retained recipient draft");
            assert_eq!(input.draft_key, draft_key);
            assert_eq!(input.row_send.as_ref().unwrap().target, recipient);
        }
    });
}
