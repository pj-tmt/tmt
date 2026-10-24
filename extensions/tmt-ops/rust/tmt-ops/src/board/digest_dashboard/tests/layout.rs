//! Current Digest layout, UUID routing and held-evidence capture scenes.
use super::*;
use crate::{
    board::{
        app::{App, Effect},
        shell::{OpsTab, RightTab},
        usage_report::{
            Delivery, Report,
            model::{HourlySeries, Pair, Seat, Traffic, Who},
        },
    },
    config::TokRange,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn fixture() -> App {
    let mut app = board();
    app.shell.tab = OpsTab::Digest;
    app.labels = supplied(serde_json::json!(3), false);
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.preferences.tok_range = TokRange::Day;
    let template = snapshot.roster.members[MEMBER].clone();
    let room = snapshot.roster.squads[0].room_id.clone();
    for i in 1..=6 {
        let id = format!("40000000-0000-4000-8000-{i:012}");
        let mut member = template.clone();
        member.id = id.clone();
        member.name = format!("agent-{i}");
        snapshot.roster.members.insert(id.clone(), member);
        snapshot.roster.squads[0].members.push(id.clone());
        snapshot.rows.insert(
            OpsTarget::Member {
                room_id: Some(room.clone()),
                identity_id: id.clone(),
            },
            serde_json::json!({"id":id,"name":format!("agent-{i}"),"state":"working"}),
        );
    }
    let ids: Vec<_> = snapshot.roster.members.keys().cloned().collect();
    snapshot.identity_ids = ids.clone().into();
    let seats = ids
        .iter()
        .enumerate()
        .map(|(i, id)| Seat {
            who: Who {
                id: Some(id.clone()),
                name: None,
                retired: false,
            },
            model_inputs: None,
            input: (i as u64 + 1) * 1000,
            output: 100,
            cached: 0,
            covered_ms: 86_400_000,
            window_ms: 86_400_000,
            from_ms: 100_000_000,
            to_ms: 186_400_000,
            observed_at_ms: 186_400_000,
            starts_at_ms: Some(100_000_000),
            complete: true,
            series: HourlySeries::new(
                100_000_000,
                std::array::from_fn(|hour| hour as u64 * (i as u64 + 1)),
                [true; 24],
            ),
        })
        .collect();
    let flow = Traffic::from_pairs(
        100_000_000,
        186_400_000,
        ids.windows(2).enumerate().map(|(i, ids)| Pair {
            sender: Who {
                id: Some(ids[0].clone()),
                name: None,
                retired: false,
            },
            recipient: Who {
                id: Some(ids[1].clone()),
                name: None,
                retired: false,
            },
            kind: "talk".into(),
            count: (i as u64 + 1) * 7,
        }),
    )
    .unwrap();
    app.usage_report.apply(Delivery {
        since: TokRange::Day,
        result: Ok(Report {
            since: TokRange::Day,
            observed_at_ms: 186_400_000,
            window_from_ms: Some(100_000_000),
            seats,
            unavailable: vec![],
            exchange: Some(flow),
            warnings: vec![],
        }),
    });
    app.workspace_reconcile();
    app
}
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn painted_matrix(app: &mut App, width: u16) -> String {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    let area = app.conversation.borrow().pane.unwrap();
    let buffer = terminal.backend().buffer();
    (area.y..area.bottom().min(area.y + 9))
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn digest_matrix_ranks_busy_low_token_agents_before_the_visible_cap() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = fixture();
    app.workspace.selected = Some(OpsTarget::DigestAll);
    let rows = app.workspace_rows();
    let busy = &rows[rows.len() - 1];
    let peer = &rows[rows.len() - 2];
    assert!(
        rows.len() > 7,
        "both busy agents must start below the first five TOK rows"
    );
    let id = |row: &crate::board::workspace::OpsTreeRow| {
        super::super::metrics::id(row).unwrap().to_owned()
    };
    let who = |id| Who {
        id: Some(id),
        name: None,
        retired: false,
    };
    let mut report = super::super::metrics::report(&app).unwrap().clone();
    report.exchange = Some(
        Traffic::from_pairs(
            report.window_from_ms.unwrap(),
            report.observed_at_ms,
            [Pair {
                sender: who(id(busy)),
                recipient: who(id(peer)),
                kind: "talk".into(),
                count: 99,
            }],
        )
        .unwrap(),
    );
    app.usage_report.apply(Delivery {
        since: TokRange::Day,
        result: Ok(report),
    });
    app.workspace_reconcile();
    let shown = painted_matrix(&mut app, 160);
    assert!(
        shown.contains(&busy.title),
        "late busy agent missing: {shown}"
    );
    assert!(
        shown.contains(&peer.title),
        "late busy peer missing: {shown}"
    );
    assert!(
        shown.contains("99"),
        "held traffic must actually paint: {shown}"
    );
    assert!(!shown.contains("Common pool") && !shown.contains("product"));
}

#[test]
fn digest_matrix_empty_flow_is_explicit_and_unavailable_is_not_zero() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = fixture();
    app.workspace.selected = Some(OpsTarget::DigestAll);
    let mut report = super::super::metrics::report(&app).unwrap().clone();
    report.exchange =
        Some(Traffic::from_pairs(0, report.observed_at_ms, Vec::<Pair>::new()).unwrap());
    app.usage_report.apply(Delivery {
        since: TokRange::Day,
        result: Ok(report.clone()),
    });
    let empty = painted_matrix(&mut app, 160);
    assert!(empty.contains("No exchanges in this range"), "{empty}");
    report.exchange = None;
    app.usage_report.apply(Delivery {
        since: TokRange::Day,
        result: Ok(report),
    });
    let unknown = painted_matrix(&mut app, 160);
    assert!(!unknown.contains("No exchanges in this range"), "{unknown}");
    assert!(
        unknown.contains('—'),
        "unavailable observations stay unknown: {unknown}"
    );
}
#[test]
fn table_has_read_only_all_and_stable_ranked_uuids_and_real_member_enters_agents_talk() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = fixture();
    let rows = app.workspace_rows();
    assert_eq!(rows[0].target, OpsTarget::DigestAll);
    assert!(
        rows[1..]
            .windows(2)
            .all(|pair| pair[0].usage >= pair[1].usage)
    );
    app.workspace.selected = Some(OpsTarget::DigestAll);
    app.workspace_reconcile();
    paint(&mut app, 160, 40);
    assert!(app.workspace_message_target().is_none());
    assert!(matches!(key(&mut app, KeyCode::Enter), Effect::None));
    assert_eq!(app.shell.tab, OpsTab::Digest);
    assert!(app.input.is_none());
    app.workspace.selected = Some(rows[1].target.clone());
    app.workspace_reconcile();
    app.right_tab = RightTab::Measure;
    paint(&mut app, 160, 40);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert_eq!(app.right_tab, RightTab::Talk);
    assert_eq!(app.workspace.selected, Some(rows[1].target.clone()));
    assert_eq!(app.focused_pane(), Some(crate::config::Pane::Replies));
}

#[test]
fn cursor_and_viewport_changes_reuse_ranked_facts_without_freezing_live_supplier_text() {
    use std::rc::Rc;
    let _no_commands = crate::runner::forbid_commands();
    let mut app = fixture();
    app.workspace.selected = Some(OpsTarget::DigestAll);
    app.workspace_reconcile();
    let before = app.workspace_rows();
    let ranked = super::super::exchange_participants(&app);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.workspace.selected, Some(before[1].target.clone()));
    paint(&mut app, 160, 40);
    assert!(Rc::ptr_eq(&before, &app.workspace_rows()));
    assert!(Rc::ptr_eq(
        &ranked,
        &super::super::exchange_participants(&app)
    ));
    paint(&mut app, 80, 32);
    assert!(Rc::ptr_eq(&before, &app.workspace_rows()));
    app.labels = supplied(serde_json::json!(9), false);
    let shown = paint(&mut app, 160, 40);
    assert!(
        shown.contains("9 observed held"),
        "supplier facts must stay live, including unknown member coverage: {shown}"
    );
    assert!(Rc::ptr_eq(&before, &app.workspace_rows()));
    assert_eq!(app.workspace_rows()[0].target, OpsTarget::DigestAll);
    assert!(app.workspace_message_target().is_some());
}

#[test]
fn a_new_usage_receipt_at_the_same_clock_reorders_rows_and_query_scope_range_reconcile() {
    use std::rc::Rc;
    let mut app = fixture();
    let before = app.workspace_rows();
    let mut report = super::super::metrics::report(&app).unwrap().clone();
    let id = report.seats[0].who.id.clone().unwrap();
    report.seats[0].input = 1_000_000;
    app.usage_report.apply(Delivery {
        since: TokRange::Day,
        result: Ok(report),
    });
    let after = app.workspace_rows();
    assert!(!Rc::ptr_eq(&before, &after));
    assert!(
        matches!(&after[1].target, OpsTarget::Member { identity_id, .. } if identity_id == &id)
    );
    assert_eq!(after[1].usage, Some(1_000_100));
    app.search = "agent-1".into();
    app.workspace_reconcile();
    let searched = app.workspace_rows();
    let ranked = super::super::exchange_participants(&app);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].name, "agent-1");
    assert_eq!(searched.len(), 2);
    assert_eq!(searched[1].title, "agent-1");
    app.workspace.visibility.scope = vec!["not-an-admitted-room".into()];
    assert_eq!(app.workspace_rows().len(), 1, "only read-only all remains");
    assert!(super::super::exchange_participants(&app).is_empty());
    app.workspace.visibility.scope.clear();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .tok_range = TokRange::Hour;
    let changed_range = app.workspace_rows();
    assert_eq!(
        changed_range[1].usage, None,
        "a different range cannot reuse Day usage"
    );
    app.search.clear();
    app.workspace_reconcile();
    assert_eq!(app.workspace_rows().len(), before.len());
}

#[test]
fn retained_canonical_rows_do_not_replace_an_explicit_membership_or_a_removed_target() {
    use std::rc::Rc;
    let mut app = fixture();
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut alternate = snapshot.roster.squads[0].clone();
    alternate.room_id = "zz-alternate-room".into();
    alternate
        .member_states
        .entry(MEMBER.into())
        .or_default()
        .state = Some("blocked".into());
    let target = OpsTarget::Member {
        room_id: Some(alternate.room_id.clone()),
        identity_id: MEMBER.into(),
    };
    snapshot.rows.insert(
        target.clone(),
        serde_json::json!({"id":MEMBER,"name":"worker","state":"blocked"}),
    );
    snapshot.roster.squads.push(alternate);
    app.workspace_reconcile();
    let canonical = app.workspace_rows();
    app.workspace.selected = Some(target.clone());
    let explicit = app.workspace_rows();
    let row = explicit.iter().find(|row| row.target == target).unwrap();
    assert_eq!(row.state.as_deref(), Some("blocked"));
    assert_eq!(row.depth, 0);
    assert!(app.workspace_message_target().is_some());
    app.workspace.selected = Some(OpsTarget::DigestAll);
    assert!(Rc::ptr_eq(&canonical, &app.workspace_rows()));
    app.workspace.selected = Some(target.clone());
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot
        .roster
        .squads
        .retain(|squad| squad.room_id != "zz-alternate-room");
    snapshot.rows.remove(&target);
    app.workspace_reconcile();
    assert_eq!(app.workspace.selected, Some(target));
    assert!(
        app.workspace_message_target().is_none(),
        "removal must not acquire a different occurrence"
    );
    assert!(!Rc::ptr_eq(&canonical, &app.workspace_rows()));
}
#[test]
fn narrow_table_keeps_24_hours_and_tab_exposes_exchange_matrix_without_selection_change() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = fixture();
    let selected = app.workspace.selected.clone();
    let shown = paint(&mut app, 80, 40);
    assert!(shown.contains("ACTIVITY · last 1d"));
    assert!(!shown.contains("SHARE"));
    assert!(!shown.contains("≈ $ in"));
    assert!(shown.contains("Details"));
    key(&mut app, KeyCode::Tab);
    let shown = paint(&mut app, 80, 40);
    assert!(shown.contains("from\\to"));
    assert!(shown.contains("Exchanges"));
    assert_eq!(app.workspace.selected, selected);
    key(&mut app, KeyCode::Esc);
    let shown = paint(&mut app, 60, 40);
    assert!(shown.contains("1d "));
    assert!(!shown.contains("ACTIVITY"));
}
#[test]
fn short_range_never_relabels_supplier_savings_and_absent_digest_keeps_core_evidence() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = fixture();
    app.labels = Default::default();
    let shown = paint(&mut app, 160, 40);
    assert!(shown.contains("28.7k"), "{shown}");
    assert!(shown.contains("Install: tmt extension install digest"));
    assert!(shown.contains("from\\to"));
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .tok_range = TokRange::Hour;
    let shown = paint(&mut app, 160, 40);
    assert!(shown.contains("Range 1h"));
    assert!(shown.contains("unavailable for this range"));
}

#[test]
fn digest_details_toggle_uses_config_cas_and_usage_controls_have_the_existing_owner() {
    use crate::board::app::Request;
    let _no_commands = crate::runner::forbid_commands();
    let mut app = fixture();
    app.workspace.selected = Some(OpsTarget::DigestAll);
    app.workspace_reconcile();
    paint(&mut app, 160, 40);
    let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) =
        key(&mut app, KeyCode::Char('e'))
    else {
        panic!("details toggle must use the existing saved preference");
    };
    assert!(expected.digest_panels.is_none());
    assert!(!next.digest_panels.unwrap().effective().side_shown());
    let mut app = fixture();
    paint(&mut app, 160, 40);
    key(&mut app, KeyCode::Char('u'));
    paint(&mut app, 160, 40);
    assert!(app.usage_chart.borrow().focused());
    key(&mut app, KeyCode::Enter);
    paint(&mut app, 160, 40);
    assert!(app.usage_controls);
    assert!(app.input.is_none());
}

#[test]
fn input_estimate_needs_attributed_input_and_a_held_quote_and_incomplete_zero_is_unknown() {
    let mut app = valued_board();
    let report = super::super::metrics::report(&fixture()).unwrap().clone();
    let mut report = report;
    report
        .seats
        .retain(|seat| seat.who.id.as_deref() == Some(MEMBER));
    let seat = &mut report.seats[0];
    seat.input = 1_000_000;
    seat.model_inputs = Some(vec![("gpt-6-sol".into(), 1_000_000)]);
    assert_eq!(
        super::super::metrics::input_price(&app, Some(&report), &[MEMBER]),
        Some(2_000_000)
    );
    let mut shifted = report.seats[0].clone();
    shifted.who.id = Some("zz-second".into());
    shifted.from_ms += 1;
    shifted.to_ms += 1;
    report.seats.push(shifted);
    assert_eq!(
        super::super::metrics::input_price(&app, Some(&report), &[MEMBER, "zz-second"]),
        None,
        "a price cannot combine two source windows"
    );
    report.seats.pop();
    report.seats[0].complete = false;
    assert_eq!(
        super::super::metrics::input_price(&app, Some(&report), &[MEMBER]),
        None,
        "incomplete usage must not look like a complete input price"
    );
    report.seats[0].complete = true;
    report.seats[0].model_inputs = None;
    assert_eq!(
        super::super::metrics::input_price(&app, Some(&report), &[MEMBER]),
        None
    );
    report.seats[0].model_inputs = Some(vec![("unpriced".into(), 1_000_000)]);
    assert_eq!(
        super::super::metrics::input_price(&app, Some(&report), &[MEMBER]),
        None
    );
    app.labels = crate::labels::Supplied::from_document("digest", &valued_document(), true);
    assert_eq!(
        super::super::metrics::input_price(&app, Some(&report), &[MEMBER]),
        None
    );
    report.seats[0].input = 0;
    report.seats[0].output = 0;
    report.seats[0].complete = false;
    assert_eq!(
        super::super::metrics::tokens(Some(&report), &[MEMBER]),
        None
    );
    report.seats[0].complete = true;
    assert_eq!(
        super::super::metrics::tokens(Some(&report), &[MEMBER]),
        Some((0, false))
    );
}

#[test]
#[ignore = "task-owned rich Digest look packet; no frozen bytes or source reads on paint"]
fn capture_digest_layout_packet() {
    use ratatui::{Terminal, backend::TestBackend};
    use tmt_cli_style::{Base, Depth, Theme};
    let output = std::env::var("TMT_DIGEST_LAYOUT_CAPTURE").expect("task-owned packet path");
    let _no_commands = crate::runner::forbid_commands();
    let mut frames = Vec::new();
    crate::status::with_now_ms(187_000_000, || {
        for (look_name, base, depth) in [
            ("dark", Base::Tmt, Depth::TrueColor),
            ("light", Base::TmtLight, Depth::TrueColor),
            ("ansi16", Base::Tmt, Depth::Ansi16),
            ("no-color", Base::Tmt, Depth::None),
        ] {
            for width in [160, 100, 80, 60] {
                for scene in [
                    "member", "priced", "all", "matrix", "missing", "empty", "stale", "loading",
                    "tiny",
                ] {
                    let mut app = fixture();
                    let view = app.view.as_mut().unwrap();
                    view.look = crate::look::Look {
                        theme: Theme::new(base),
                        depth,
                    };
                    view.home.as_mut().unwrap().workspace.squad_looks.clear();
                    match scene {
                        "priced" => {
                            app.labels = crate::labels::Supplied::from_document(
                                "digest",
                                &valued_document(),
                                false,
                            );
                            let mut report = super::super::metrics::report(&app).unwrap().clone();
                            for seat in &mut report.seats {
                                seat.model_inputs = Some(vec![("gpt-6-sol".into(), seat.input)]);
                            }
                            app.usage_report.apply(Delivery {
                                since: TokRange::Day,
                                result: Ok(report),
                            });
                        }
                        "all" => app.workspace.selected = Some(OpsTarget::DigestAll),
                        "matrix" => app.focus = 1,
                        "missing" => app.labels = Default::default(),
                        "loading" => app.usage_report = Default::default(),
                        "empty" => {
                            let snapshot =
                                &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                            snapshot.roster.members.clear();
                            snapshot.roster.squads.clear();
                            snapshot.rows.clear();
                            snapshot.identity_ids = std::sync::Arc::from([]);
                            app.labels = Default::default();
                        }
                        "stale" => app.usage_report.apply(Delivery {
                            since: TokRange::Day,
                            result: Err("synthetic refresh failure".into()),
                        }),
                        _ => {}
                    }
                    app.workspace_reconcile();
                    let height = if scene == "tiny" { 8 } else { 40 };
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| {
                            crate::board::view::prepare(&mut app, frame.area());
                            crate::board::view::render(frame, &app);
                        })
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let lines: Vec<_> = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect();
                    frames.push(serde_json::json!({"scene":scene,"look":look_name,"width":width,"height":height,"lines":lines,"cells":buffer.content.iter().map(|cell|serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>(),"rows":format!("{:?}",app.workspace.selected),"hits":format!("{:?}",app.hits.borrow()),"stripHits":format!("{:?}",app.workspace_strip_hits.borrow())}));
                }
            }
        }
    });
    std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
}

#[test]
fn mixed_batch_windows_and_missing_agents_cannot_manufacture_an_aggregate_or_zero_hour() {
    let app = fixture();
    let mut report = super::super::metrics::report(&app).unwrap().clone();
    let ids: Vec<_> = report
        .seats
        .iter()
        .take(2)
        .map(|seat| seat.who.id.clone().unwrap())
        .collect();
    let refs: Vec<_> = ids.iter().map(String::as_str).collect();
    assert!(super::super::metrics::tokens(Some(&report), &refs).is_some());
    report.seats[1].from_ms += 300_000;
    report.seats[1].to_ms += 300_000;
    assert_eq!(super::super::metrics::tokens(Some(&report), &refs), None);
    assert_eq!(
        super::super::metrics::activity(Some(&report), &[&ids[0], "unavailable"])[0],
        None
    );
    assert!(super::super::metrics::activity(Some(&report), &[&ids[0], "unavailable"])[1].is_some());
    let busiest = super::super::metrics::busiest(&report, &ids[0]).unwrap();
    assert_eq!(busiest.from_ms, 100_000_000 + 23 * 3_600_000);
    assert_eq!(busiest.to_ms, 100_000_000 + 24 * 3_600_000);
    report.seats[1].series.from_ms += 3_600_000;
    assert!(
        super::super::metrics::activity(Some(&report), &refs)
            .iter()
            .all(Option::is_none)
    );
}

#[test]
#[ignore = "local release-mode Digest event/draw evidence; not a CI threshold"]
fn digest_event_draw_timing() {
    if cfg!(debug_assertions) {
        panic!("run the Digest probe with --release");
    }
    use super::super::metrics;
    use crate::board::timing::probe::{measure, report};
    use ratatui::{Terminal, backend::TestBackend};
    crate::status::with_now_ms(187_000_000, || {
        for count in [150, 1_000, 10_000] {
            let mut app = crate::board::app::workspace::tests::populated_board(36, count);
            app.shell.tab = crate::board::shell::OpsTab::Digest;
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .preferences
                .tok_range = crate::config::TokRange::Day;
            let template = metrics::report(&fixture()).unwrap().seats[0].clone();
            let ids: Vec<_> = app
                .workspace_snapshot()
                .unwrap()
                .roster
                .members
                .keys()
                .cloned()
                .collect();
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .identity_ids = ids.clone().into();
            let seats = ids
                .iter()
                .enumerate()
                .map(|(index, id)| {
                    let mut seat = template.clone();
                    seat.who.id = Some(id.clone());
                    seat.input = index as u64 + 1;
                    seat
                })
                .collect();
            app.usage_report
                .apply(crate::board::usage_report::Delivery {
                    since: crate::config::TokRange::Day,
                    result: Ok(crate::board::usage_report::Report {
                        since: crate::config::TokRange::Day,
                        observed_at_ms: 186_400_000,
                        window_from_ms: Some(100_000_000),
                        seats,
                        unavailable: vec![],
                        exchange: None,
                        warnings: vec![],
                    }),
                });
            app.workspace_reconcile();
            let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
            let _no_commands = crate::runner::forbid_commands();
            for _ in 0..10 {
                measure(&mut app, &mut terminal, |_, _| {});
            }
            report(
                count,
                "digest-steady",
                (0..100)
                    .map(|_| measure(&mut app, &mut terminal, |_, _| {}))
                    .collect(),
            );
            report(
                count,
                "digest-row-step",
                (0..100)
                    .map(|_| {
                        measure(&mut app, &mut terminal, |app, _| {
                            assert_eq!(
                                app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                                Effect::None
                            );
                        })
                    })
                    .collect(),
            );
        }
    });
}

#[test]
fn input_price_qualification_is_shown_once_by_the_visible_amount() {
    let _no_commands = crate::runner::forbid_commands();
    for width in [160, 100, 80, 60] {
        let mut app = fixture();
        let shown = paint(&mut app, width, 60);
        assert_eq!(
            shown
                .matches("Input tokens at list price; not a bill.")
                .count(),
            1,
            "width={width}: {shown}"
        );
        if width < 100 {
            assert!(shown.contains("Input list-price estimate: —"));
        }
        // The columns a narrow pane drops are named once; a pane that has them says nothing.
        assert_eq!(
            shown.matches(super::super::table::NARROW_HINT).count(),
            usize::from(!super::super::table::price_column_shown(&app)),
            "width={width}: {shown}"
        );
    }
}

#[test]
fn painted_digest_range_click_uses_the_shared_preferences_save() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let _no_commands = crate::runner::forbid_commands();
    let mut app = fixture();
    paint(&mut app, 160, 36);
    let hit = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find(|(_, choice)| matches!(choice, crate::board::app::Choice::TokRange(_)))
        .unwrap()
        .0;
    let effect = app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    let Effect::Act(crate::board::app::Request::WorkspacePreferences { expected, next, .. }) =
        effect
    else {
        panic!("Range click must use the workspace preferences writer");
    };
    assert_eq!(expected.tok_range, crate::config::TokRange::Day);
    assert_eq!(next.tok_range, crate::config::TokRange::Week);
    assert_eq!(
        app.tok_range(),
        crate::config::TokRange::Day,
        "only a confirmed save changes the held preference"
    );
}

#[test]
fn digest_selected_agent_takes_last_available_slot_without_reordering_cached_traffic() {
    let mut app = fixture();
    let ranked = super::super::exchange_participants(&app);
    let selected = ranked.last().unwrap().id.clone();
    let row = app
        .workspace_rows()
        .iter()
        .position(|row| super::super::metrics::id(row) == Some(selected.as_str()))
        .unwrap();
    app.select(row);
    for slots in [1, 2, 3, 5] {
        let visible = crate::board::exchange_matrix::participants(&ranked, Some(&selected), slots);
        assert_eq!(visible.len(), slots);
        assert_eq!(visible.last().unwrap().id, selected);
        assert_eq!(&visible[..slots - 1], &ranked[..slots - 1]);
    }
    assert!(std::rc::Rc::ptr_eq(
        &ranked,
        &super::super::exchange_participants(&app)
    ));
}

/// Additive evidence uses the same held-report fixture as the rank regressions.
pub(in crate::board) fn capture_case(state: &str, look: crate::look::Look) -> App {
    let mut app = fixture();
    app.view.as_mut().unwrap().look = look;
    for style in app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .values_mut()
    {
        *style = look;
    }
    app.shell.tab = OpsTab::Agents;
    key(&mut app, KeyCode::Char('2'));
    match state {
        "digest-first-open" => {}
        "digest-retained-pick" => {
            let last = app.workspace_rows().len() - 1;
            app.select(last);
            let selected = app.workspace.selected.clone();
            key(&mut app, KeyCode::Char('1'));
            key(&mut app, KeyCode::Char('2'));
            assert_eq!(app.workspace.selected, selected);
        }
        "digest-busy-late" | "digest-empty-flow" | "digest-unavailable-flow" => {
            let mut report = super::super::metrics::report(&app).unwrap().clone();
            let pairs = if state == "digest-busy-late" {
                let rows = app.workspace_rows();
                let who = |row: &crate::board::workspace::OpsTreeRow| Who {
                    id: Some(super::super::metrics::id(row).unwrap().into()),
                    name: None,
                    retired: false,
                };
                vec![Pair {
                    sender: who(&rows[rows.len() - 1]),
                    recipient: who(&rows[rows.len() - 2]),
                    kind: "talk".into(),
                    count: 99,
                }]
            } else {
                vec![]
            };
            report.exchange = (state != "digest-unavailable-flow")
                .then(|| Traffic::from_pairs(100_000_000, 186_400_000, pairs).unwrap());
            app.usage_report.apply(Delivery {
                since: TokRange::Day,
                result: Ok(report),
            });
        }
        "digest-no-agents" => {
            let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
            snapshot.roster.members.clear();
            snapshot.roster.squads.clear();
            snapshot.rows.clear();
            snapshot.identity_ids = vec![].into();
            app.workspace_reconcile();
        }
        _ => panic!("unknown Digest evidence scene {state}"),
    }
    app
}
