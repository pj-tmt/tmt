//! Current Digest layout, UUID routing and held-evidence capture scenes.
use super::*;
use crate::{
    board::{
        app::{App, Effect},
        shell::{OpsTab, RightTab},
        usage_report::{
            Delivery, Report,
            model::{HourlySeries, Pair, Point, Seat, Traffic, Who},
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
            series: HourlySeries {
                from_ms: 100_000_000,
                points: (0..24)
                    .map(|hour| Point {
                        tokens: hour * (i as u64 + 1),
                        complete: true,
                    })
                    .collect(),
            },
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
fn narrow_table_keeps_24_hours_and_tab_exposes_exchange_matrix_without_selection_change() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = fixture();
    let selected = app.workspace.selected.clone();
    let shown = paint(&mut app, 80, 40);
    assert!(shown.contains("ACTIVITY · last 24h"));
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
    assert!(shown.contains("24h "));
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
    }
}
