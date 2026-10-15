use super::model;
use crate::board::app::workspace::tests::{MEMBER, board};
use crate::board::workspace::{
    OpsMember, OpsObservation, OpsTarget, observe::OpsWorkspaceSnapshot,
};

fn supplied(held: serde_json::Value, stale: bool) -> crate::labels::Supplied {
    crate::labels::Supplied::from_document(
        "digest",
        &serde_json::json!({
            "version": 1, "observedAtMs": 1000,
            "members": [{"identityId":MEMBER, "name":"Agent",
                "mode":"auto", "intervalMs":null, "settingSource":"default", "settingValue":"default",
                "heldCount":held, "statsObservedAtMs":null, "nextSendAtMs":null, "timeToSendMs":null,
            "labels":[{"text":"Auto","colorClass":"text", "action": {
                "kind":"choose", "argv":["digest",MEMBER,"{value}"],
                "options":[{"label":"Default (Auto)","value":"default"},
                    {"label":"Auto","value":"auto"},{"label":"Off","value":"off"}],
                "current":"default"
            }},{"text":"When idle","colorClass":"muted"}]
            }]
        }),
        stale,
    )
}

#[test]
fn null_stats_keep_the_inherited_idle_setting_without_a_zero_count() {
    let supplied = supplied(serde_json::Value::Null, false);
    assert!(model::summary(&supplied, None).contains("held unavailable"));
    assert!(!model::summary(&supplied, None).contains("0 held"));
    let details = model::details(&supplied, MEMBER);
    assert!(details.iter().any(|s| s.contains("inherited default")));
    assert!(details.iter().any(|s| s == "Auto: when idle"));
    assert!(!details.iter().any(|s| s.starts_with("Next send")));
}

#[test]
fn known_zero_and_retained_stale_statistics_stay_distinct() {
    let zero = supplied(serde_json::json!(0), false);
    assert!(model::summary(&zero, None).contains("0 held"));
    let stale = supplied(serde_json::json!(3), true);
    assert!(model::summary(&stale, None).contains("3 held · stale observation"));
    assert!(
        model::details(&stale, MEMBER)
            .iter()
            .any(|s| s.contains("Stale observation"))
    );
}

#[test]
fn unavailable_statistics_never_become_zero() {
    assert_eq!(
        model::summary(&Default::default(), None),
        "Digest status unavailable"
    );
    assert!(
        model::details(&Default::default(), "missing")
            .iter()
            .any(|line| line.contains("unavailable"))
    );
}

#[test]
fn duplicate_memberships_have_one_row_and_keep_a_management_context() {
    let mut snapshot = OpsWorkspaceSnapshot::default();
    snapshot.roster.observation = OpsObservation::Complete;
    snapshot.roster.members.insert(
        "id".into(),
        OpsMember {
            id: "id".into(),
            name: "Agent".into(),
            state: None,
            waiting: false,
            presence: None,
            usage: None,
        },
    );
    snapshot.roster.membership_complete = true;
    for room in ["a", "b"] {
        snapshot
            .roster
            .squads
            .push(crate::board::workspace::OpsSquad {
                room_id: room.into(),
                name: room.into(),
                members: vec!["id".into()],
                lead: None,
                member_states: Default::default(),
                observation: OpsObservation::Complete,
            });
    }
    for room in [None, Some("a"), Some("b")] {
        snapshot.rows.insert(
            OpsTarget::Member {
                room_id: room.map(str::to_owned),
                identity_id: "id".into(),
            },
            serde_json::json!({"id":"id","name":"Agent"}),
        );
    }
    let rows =
        crate::board::workspace::identity_rows(&snapshot, "agent", None, &Default::default());
    assert_eq!(rows.len(), 1);
    assert!(
        matches!(&rows[0].target, OpsTarget::Member { room_id: Some(room), .. } if room == "a")
    );
    assert!(
        crate::board::workspace::identity_rows(&snapshot, "absent", None, &Default::default())
            .is_empty()
    );
    let preferred = OpsTarget::Member {
        room_id: Some("b".into()),
        identity_id: "id".into(),
    };
    assert_eq!(
        crate::board::workspace::identity_rows(
            &snapshot,
            "agent",
            Some(&preferred),
            &Default::default()
        )[0]
        .target,
        preferred
    );
    snapshot.roster.squads[1].members.clear();
    assert_ne!(
        crate::board::workspace::identity_rows(
            &snapshot,
            "agent",
            Some(&preferred),
            &Default::default()
        )[0]
        .target,
        preferred
    );
    snapshot.roster.squads[0].members.clear();
    snapshot.rows.remove(&OpsTarget::Member {
        room_id: None,
        identity_id: "id".into(),
    });
    assert!(
        crate::board::workspace::identity_rows(&snapshot, "agent", None, &Default::default())
            .is_empty(),
        "stale row documents do not establish membership"
    );
}

fn paint(app: &mut crate::board::app::App, width: u16, height: u16) -> String {
    use ratatui::{Terminal, backend::TestBackend};
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

fn press(app: &mut crate::board::app::App, key: char) -> crate::board::app::Effect {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
}

#[test]
fn shell_dashboard_keeps_the_same_identity_and_uses_the_supplied_picker() {
    use crate::board::app::{Effect, Request};
    let mut app = board();
    let _no_commands = crate::runner::forbid_commands();
    app.labels = supplied(serde_json::json!(3), false);
    let selected = app.workspace.selected.clone();
    press(&mut app, '3');
    assert_eq!(app.shell.tab, crate::board::shell::OpsTab::Digest);
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.workspace_rows().len(), 1);
    let shown = paint(&mut app, 160, 36);
    assert!(shown.contains("Digest"));
    assert!(shown.contains("Auto ▾"));
    assert!(shown.contains("inherited default"));
    assert!(shown.contains("Auto: when idle"));
    match press(&mut app, 'D') {
        Effect::Act(Request::DigestOpen(ask)) => {
            assert_eq!(ask.squad, "product");
            assert_eq!(ask.member, "worker");
            assert_eq!(ask.choose.current, "default");
            assert_eq!(ask.choose.argv("off").unwrap(), ["digest", MEMBER, "off"]);
        }
        effect => panic!("expected supplier-owned picker admission, got {effect:?}"),
    }
}

#[test]
fn status_loss_and_low_height_do_not_leave_old_chip_pointer_authority() {
    let mut app = board();
    app.labels = supplied(serde_json::json!(3), false);
    press(&mut app, '3');
    paint(&mut app, 160, 36);
    assert!(!app.chip_hits.borrow().is_empty());
    let selected = app.workspace.selected.clone();
    app.labels = Default::default();
    app.invalidate_frame_geometry();
    let shown = paint(&mut app, 60, 8);
    assert_eq!(app.workspace.selected, selected);
    assert!(shown.contains("unavailable"));
    assert!(app.chip_hits.borrow().is_empty());
    paint(&mut app, 80, 18);
    assert!(app.chip_hits.borrow().is_empty());
    assert!(app.digest_choice(app.selected_row().unwrap()).is_none());
}

fn quota_board() -> crate::board::app::App {
    let mut app = board();
    app.view.as_mut().unwrap().home_rate.insert(
        "product".into(),
        crate::board::app::RateView {
            input: crate::board::rate::Input {
                room: super::super::app::workspace::tests::ROOM.into(),
                resumes: Default::default(),
                names: Default::default(),
            },
            settings: crate::config::TokenRate {
                enabled: true,
                ..Default::default()
            },
            history: None,
        },
    );
    app
}

#[test]
fn quota_details_open_from_current_cells_and_menu_without_retargeting_or_replaying() {
    use crate::board::app::{Choice, Effect, HitTarget};
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    let mut app = quota_board();
    app.shell.tab = crate::board::shell::OpsTab::Digest;
    app.limits = Some(vec![crate::board::limits::Standing {
        provider: crate::board::limits::Provider::named("claude"),
        latest: None,
        burn: None,
        trend: None,
    }]);
    let selected = app.workspace.selected.clone();
    paint(&mut app, 100, 36);
    let hit = app
        .hits
        .borrow()
        .iter()
        .find(|hit| hit.target == HitTarget::Quota)
        .copied()
        .unwrap();
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.x,
        row: hit.y,
        modifiers: KeyModifiers::NONE,
    };
    app.invalidate_frame_geometry();
    app.mouse(click, std::time::Instant::now());
    assert!(app.quota_details.is_none(), "old cells have no authority");
    paint(&mut app, 100, 36);
    assert!(matches!(
        app.mouse(click, std::time::Instant::now()),
        Effect::None
    ));
    assert_eq!(app.workspace.selected, selected);
    assert!(app.reload_blocked());
    let shown = paint(&mut app, 80, 18);
    assert!(shown.contains("Subscriptions"));
    assert!(shown.contains("unavailable or stale"));
    assert!(
        !app.frame_clocks
            .borrow()
            .iter()
            .any(|clock| matches!(clock, crate::board::view::ClockSurface::Limits { .. })),
        "the covered Digest body contributes no quota clocks"
    );
    assert!(!shown.contains("0% remaining"));
    assert!(matches!(
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Effect::None
    ));
    assert!(app.quota_details.is_none());
    assert_eq!(app.workspace.selected, selected);
    assert!(matches!(app.context_menu(), Effect::None));
    let entry = app
        .menu
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.label == "Quota details")
        .unwrap()
        .choice
        .clone();
    assert!(matches!(entry, Choice::Action(_)));
    app.choose(entry);
    assert!(app.quota_details.is_some());
    assert_eq!(app.workspace.selected, selected);
    assert!(
        matches!(
            app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Effect::None
        ),
        "close is not board quit"
    );
}

#[test]
fn quota_detail_scroll_uses_current_modal_viewport_and_resize_revokes_it() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    let mut app = board();
    app.shell.tab = crate::board::shell::OpsTab::Digest;
    app.limits = Some(
        (0..12)
            .map(|i| crate::board::limits::Standing {
                provider: crate::board::limits::Provider::named(&format!("provider{i}")),
                latest: None,
                burn: None,
                trend: None,
            })
            .collect(),
    );
    app.open_quota_details();
    paint(&mut app, 80, 18);
    let area = app
        .quota_details
        .as_ref()
        .unwrap()
        .borrow()
        .scroll
        .viewport();
    assert!(area.height > 0);
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert!(app.quota_details.as_ref().unwrap().borrow().scroll.offset() > 0);
    let selected = app.workspace.selected.clone();
    app.invalidate_frame_geometry();
    let old = app.quota_details.as_ref().unwrap().borrow().scroll.offset();
    app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(
        app.quota_details.as_ref().unwrap().borrow().scroll.offset(),
        old
    );
    paint(&mut app, 60, 8);
    assert_eq!(app.workspace.selected, selected);
    paint(&mut app, 100, 36);
    app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    let details = app.quota_details.as_ref().unwrap().borrow();
    assert_eq!(details.scroll.offset(), details.scroll.limit());
}

#[test]
fn quota_window_frames_show_partial_full_near_empty_and_unknown_in_both_shells() {
    use crate::board::{
        limits::{Provider, Reading, Standing, Window},
        shell::OpsTab,
    };
    use ratatui::{Terminal, backend::TestBackend};
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        for (theme, look) in crate::board::picker_surface::evidence::looks()
            .into_iter()
            .take(2)
            .enumerate()
        {
            for tab in [OpsTab::Overview, OpsTab::Digest] {
                for width in [160, 140, 100, 80, 60] {
                    let mut app = quota_board();
                    app.shell.tab = tab;
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .squad_looks
                        .clear();
                    app.view.as_mut().unwrap().look = look;
                    app.limits = Some(
                        [
                            (0.5, Some(97.0)),
                            (44.0, None),
                            (100.0, Some(0.0)),
                            (78.0, None),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (left, short))| Standing {
                            provider: Provider::named(&format!("provider{i}")),
                            latest: (i != 3).then_some(Reading {
                                observed_at_ms: NOW - 60_000,
                                weekly: Window {
                                    left,
                                    resets_at_ms: NOW + 345_600_000,
                                },
                                short: short.map(|left| Window {
                                    left,
                                    resets_at_ms: NOW + 10_800_000,
                                }),
                            }),
                            burn: (i != 3).then_some(0.4),
                            trend: None,
                        })
                        .collect(),
                    );
                    let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
                    terminal
                        .draw(|frame| {
                            crate::board::view::prepare(&mut app, frame.area());
                            crate::board::view::render(frame, &app);
                        })
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let shown: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
                    for expected in ["<1%", "44%", "100%", "no reading yet"] {
                        assert!(
                            shown.contains(expected),
                            "{tab:?}/{theme}/{width}: missing {expected}"
                        );
                    }
                    assert!(
                        app.hits
                            .borrow()
                            .iter()
                            .any(|hit| hit.target == crate::board::app::HitTarget::Quota)
                    );
                    if let Ok(directory) = std::env::var("TMT_QUOTA_CAPTURE_OUT") {
                        let directory = std::path::Path::new(&directory);
                        std::fs::create_dir_all(directory).unwrap();
                        let value = serde_json::json!({"width":width,"height":36,"tab":format!("{tab:?}"),"theme":if theme==0 {"tmt"} else {"tmt-light"},
                            "cells":buffer.content.iter().map(|cell|serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>(),
                            "hits":format!("{:?}",app.hits.borrow())});
                        std::fs::write(
                            directory.join(format!("{tab:?}-{theme}-{width}.json")),
                            serde_json::to_vec(&value).unwrap(),
                        )
                        .unwrap();
                    }
                    assert_eq!(app.open_quota_details(), crate::board::app::Effect::None);
                    terminal
                        .draw(|frame| crate::board::view::render(frame, &app))
                        .unwrap();
                    let details: String = terminal
                        .backend()
                        .buffer()
                        .content
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect();
                    assert!(
                        details.contains("reset 4d00h"),
                        "{tab:?}/{theme}/{width}: {details}"
                    );
                    assert!(details.contains("Weekly pace: ~0.4%/h"), "{details}");
                }
            }
        }
    });
}

#[test]
fn disabled_subscription_sampling_never_promises_an_in_flight_read() {
    let mut app = board();
    app.shell.tab = crate::board::shell::OpsTab::Digest;
    app.open_quota_details();
    let shown = paint(&mut app, 100, 36);
    assert!(shown.contains("Subscription sampling is disabled"));
    assert!(!shown.contains("Updating limits"));
    assert!(!shown.contains("0% remaining"));
}

mod performance;

#[test]
fn scoped_held_counts_exclude_other_identities_and_keep_missing_stats_unknown() {
    let source = supplied(serde_json::json!(3), false);
    let only = [MEMBER].into();
    assert_eq!(model::summary(&source, Some(&only)), "Digest · 3 held");
    let missing = ["33333333-3333-4333-8333-333333333333"].into();
    assert_eq!(
        model::summary(&source, Some(&missing)),
        "Digest · held unavailable · 1 unknown"
    );
    let partial = [MEMBER, "33333333-3333-4333-8333-333333333333"].into();
    assert_eq!(
        model::summary(&source, Some(&partial)),
        "Digest · ≥3 observed held · 1 unknown"
    );
    assert_eq!(
        model::summary(&source, Some(&Default::default())),
        "Digest · 0 held"
    );
}

#[test]
fn scoped_digest_summary_follows_its_rows_and_saved_hiding_stays_outside_digest() {
    use crate::board::{
        shell::OpsTab,
        workspace::{OpsSquad, OpsTarget},
    };
    const OUTSIDE: &str = "77777777-7777-4777-8777-777777777777";
    const OTHER: &str = "88888888-8888-4888-8888-888888888888";
    let mut app = board();
    app.labels = supplied(serde_json::json!(3), false);
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let room = snapshot.roster.squads[0].room_id.clone();
    snapshot.roster.members.insert(
        OUTSIDE.into(),
        OpsMember {
            id: OUTSIDE.into(),
            name: "outside".into(),
            state: Some("working".into()),
            waiting: false,
            presence: Some(true),
            usage: None,
        },
    );
    snapshot.roster.squads.push(OpsSquad {
        room_id: OTHER.into(),
        name: "other".into(),
        members: vec![OUTSIDE.into()],
        lead: None,
        member_states: Default::default(),
        observation: OpsObservation::Complete,
    });
    snapshot.rows.insert(
        OpsTarget::Member {
            room_id: Some(OTHER.into()),
            identity_id: OUTSIDE.into(),
        },
        serde_json::json!({"id":OUTSIDE,"name":"outside"}),
    );
    let mut preferences = snapshot.preferences.clone();
    preferences.hidden.insert(room.clone());
    snapshot.set_preferences(preferences);
    app.shell.tab = OpsTab::Digest;
    app.workspace.visibility.scope = vec![room];
    app.workspace_reconcile();
    assert_eq!(app.workspace_rows().len(), 1);
    assert_eq!(super::summary(&app), "Digest · 3 held");
    let mut captures = Vec::new();
    for (base, depth) in [
        (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
        (
            tmt_cli_style::Base::TmtLight,
            tmt_cli_style::Depth::TrueColor,
        ),
        (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
    ] {
        let view = app.view.as_mut().unwrap();
        view.look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(base),
            depth,
        };
        view.home.as_mut().unwrap().workspace.squad_looks.clear();
        for (width, height) in [(160, 44), (100, 24), (60, 24)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let lines: Vec<String> = (0..height)
                .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
                .collect();
            let text = lines.join("\n");
            assert!(
                text.contains("Digest · 3 held"),
                "the scoped summary must actually paint"
            );
            assert!(text.contains("Usage · global identities"));
            assert!(
                text.contains("Est. ctx avoided 24h: — (0 of 2)"),
                "Usage estimates keep the global roster while held counts follow scope"
            );
            assert!(text.contains("Scope:"));
            captures.push(serde_json::json!({"width":width,"height":height,"base":format!("{base:?}"),"depth":format!("{depth:?}"),"tab":"Digest","mode":"scoped","lines":lines,
                "cells":buffer.content.iter().map(|cell|serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>() }));
        }
    }
    if let Some(output) = std::env::var_os("TMT_DIGEST_SCOPE_CAPTURE") {
        std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
    }
    app.search = "outside".into();
    app.workspace_reconcile();
    assert!(app.workspace_rows().is_empty());
    assert_eq!(super::summary(&app), "Digest · 0 held");
    assert!(
        paint(&mut app, 100, 36).contains("Est. ctx avoided 24h: — (0 of 2)"),
        "scoped search must not narrow the separately labeled global Usage estimate"
    );
    app.search.clear();
    app.workspace.visibility.scope.clear();
    app.workspace_reconcile();
    assert_eq!(app.workspace_rows().len(), 2, "Digest ignores saved hiding");
    assert_eq!(super::summary(&app), "Digest · 3 held");
}

#[test]
fn savings_captions_keep_estimate_window_and_partial_coverage() {
    use crate::{
        config::{DigestWindow, SavingsUnit},
        labels::digest::SavingsSummary,
    };
    let mut value = SavingsSummary {
        tokens: Some(18_200),
        included: 3,
        total: 5,
        ..Default::default()
    };
    assert_eq!(
        model::savings_caption(&value, DigestWindow::Day, SavingsUnit::Tokens, 100),
        "Est. ctx avoided 24h: 18.2k (3 of 5)"
    );
    assert_eq!(
        model::savings_caption(&value, DigestWindow::Day, SavingsUnit::Tokens, 30),
        "Est. avoided 24h: 18.2k (3 of 5)"
    );
    value.tokens = Some(1_400_000);
    assert_eq!(
        model::savings_caption(&value, DigestWindow::AllTime, SavingsUnit::Tokens, 100),
        "Est. ctx avoided all time: 1.4M (3 of 5)"
    );
    value.tokens = Some(0);
    value.included = 5;
    assert_eq!(
        model::savings_caption(&value, DigestWindow::Week, SavingsUnit::Tokens, 100),
        "Est. ctx avoided 7d: 0"
    );
    value.tokens = None;
    value.included = 0;
    assert_eq!(
        model::savings_caption(&value, DigestWindow::Day, SavingsUnit::Tokens, 100),
        "Est. ctx avoided 24h: — (0 of 5)"
    );
}

#[test]
fn every_unit_keeps_one_label_and_changes_only_the_value() {
    use crate::{
        config::{DigestWindow, SavingsUnit},
        labels::digest::{SavingsSummary, Valuation},
    };
    let mut value = SavingsSummary {
        tokens: Some(18_200),
        included: 3,
        total: 3,
        percent: Valuation {
            value: Some(1_204),
            included: 3,
            conflict: false,
        },
        cost: Valuation {
            value: Some(12_398_600),
            included: 2,
            conflict: false,
        },
        ..Default::default()
    };
    let caption = |value: &SavingsSummary, window, unit, width| {
        model::savings_caption(value, window, unit, width)
    };
    assert_eq!(
        caption(&value, DigestWindow::Day, SavingsUnit::Tokens, 100),
        "Est. ctx avoided 24h: 18.2k"
    );
    assert_eq!(
        caption(&value, DigestWindow::Day, SavingsUnit::Percent, 100),
        "Est. ctx avoided 24h: 12% of input"
    );
    // Each unit has its own coverage.
    assert_eq!(
        caption(&value, DigestWindow::Day, SavingsUnit::Cost, 100),
        "Est. ctx avoided 24h: ≈ $12.40 at list price (2 of 3)"
    );
    // Lifetime has no percent: the saved choice reads as tokens.
    assert_eq!(
        caption(&value, DigestWindow::AllTime, SavingsUnit::Percent, 100),
        "Est. ctx avoided all time: 18.2k"
    );
    // The qualifier that stops cost reading as money saved never narrows away.
    assert!(
        caption(&value, DigestWindow::Day, SavingsUnit::Cost, 20)
            .ends_with("at list price (2 of 3)")
    );
    for (points, shown) in [
        (0, "0%"),
        (1, "<1%"),
        (49, "<1%"),
        (50, "1%"),
        (1_667, "17%"),
        (10_000, "100%"),
    ] {
        value.percent.value = Some(points);
        assert_eq!(
            caption(&value, DigestWindow::Week, SavingsUnit::Percent, 100),
            format!("Est. ctx avoided 7d: {shown} of input")
        );
    }
    for (micros, shown) in [
        (0, "$0.00"),
        (4_999, "< $0.01"),
        (5_000, "≈ $0.01"),
        (12_395_000, "≈ $12.40"),
        (1_234_567_890, "≈ $1,234.57"),
        (9_007_199_254_740_991, "≈ $9,007,199,254.74"),
    ] {
        value.cost.value = Some(micros);
        value.cost.included = 3;
        assert_eq!(
            caption(&value, DigestWindow::Day, SavingsUnit::Cost, 100),
            format!("Est. ctx avoided 24h: {shown} at list price")
        );
    }
    // Values that could not be combined are unavailable, named, and the note
    // yields to width before the segment does.
    value.percent = Valuation {
        value: None,
        included: 2,
        conflict: true,
    };
    value.cost = Valuation {
        value: None,
        included: 2,
        conflict: true,
    };
    assert_eq!(
        caption(&value, DigestWindow::Day, SavingsUnit::Percent, 100),
        "Est. ctx avoided 24h: — (intervals differ)"
    );
    assert_eq!(
        caption(&value, DigestWindow::Day, SavingsUnit::Percent, 38),
        "Est. avoided 24h: — (intervals differ)"
    );
    assert_eq!(
        caption(&value, DigestWindow::Day, SavingsUnit::Percent, 37),
        "Est. avoided 24h: —"
    );
    assert_eq!(
        caption(&value, DigestWindow::Day, SavingsUnit::Cost, 100),
        "Est. ctx avoided 24h: — (prices differ)"
    );
    // Nobody has the valuation: unavailable with its own coverage, not zero.
    value.percent = Valuation::default();
    assert_eq!(
        caption(&value, DigestWindow::Day, SavingsUnit::Percent, 100),
        "Est. ctx avoided 24h: — (0 of 3)"
    );
}

#[test]
fn lifetime_detail_uses_individual_source_starts_in_local_dates() {
    use crate::{config::DigestWindow, labels::digest::SavingsSummary};
    let ms = |value: &str| value.parse::<jiff::Timestamp>().unwrap().as_millisecond() as u64;
    let zone = jiff::tz::TimeZone::get("Asia/Tokyo").unwrap();
    let mut value = SavingsSummary {
        tokens: Some(1),
        included: 1,
        total: 1,
        earliest_from_ms: Some(ms("2026-10-01T16:00:00Z")),
        latest_from_ms: Some(ms("2026-10-08T16:00:00Z")),
        ..Default::default()
    };
    assert_eq!(
        model::savings_start(&value, DigestWindow::AllTime, &zone, 160).as_deref(),
        Some("since Digest began recording · members started 10-02 to 10-09")
    );
    value.latest_from_ms = value.earliest_from_ms;
    assert_eq!(
        model::savings_start(&value, DigestWindow::AllTime, &zone, 160).as_deref(),
        Some("since Digest began recording on 10-02")
    );
    assert_eq!(
        model::savings_start(&value, DigestWindow::Day, &zone, 160),
        None
    );
    value.earliest_from_ms = None;
    assert_eq!(
        model::savings_start(&value, DigestWindow::AllTime, &zone, 160),
        None
    );
}

fn savings_document() -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tmt-digest/contracts/vectors/status-v1.json");
    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    document = document["status"][0]["response"].clone();
    let mut row = document["members"][0].clone();
    row["identityId"] = serde_json::json!(MEMBER);
    row["labels"] = serde_json::json!([{"text":"Auto","colorClass":"text"}]);
    row["savings"] = serde_json::json!({"method":"context-input-avoided-v1", "window":"24h", "fromMs":0, "throughMs":1000, "estimatedInputTokensAvoided":18200, "complete":true,"gap":null});
    document["members"] = serde_json::json!([row]);
    document
}

fn savings_board() -> crate::board::app::App {
    let mut app = board();
    app.labels = crate::labels::Supplied::from_document("digest", &savings_document(), false);
    app
}

#[test]
fn digest_usage_shows_the_estimate_for_unique_roster_members() {
    let mut app = savings_board();
    press(&mut app, '3');
    let shown = paint(&mut app, 160, 36);
    assert!(shown.contains("Est. ctx avoided 24h: 18.2k"), "{shown}");
}

#[test]
fn member_conversation_status_shows_the_supplied_estimate() {
    let mut app = savings_board();
    let shown = paint(&mut app, 160, 36);
    assert!(shown.contains("Est. ctx avoided 24h: 18.2k"), "{shown}");
}

#[test]
fn selected_squad_summary_shows_the_same_supplied_estimate() {
    let mut app = savings_board();
    app.workspace.selected = Some(OpsTarget::Squad(
        super::super::app::workspace::tests::ROOM.into(),
    ));
    app.workspace_reconcile();
    let shown = paint(&mut app, 160, 36);
    assert!(shown.contains("Est. ctx avoided 24h: 18.2k"), "{shown}");
}

fn valued_document() -> serde_json::Value {
    let mut document = savings_document();
    document["members"][0]["savings"]["cost"] = serde_json::json!({
        "usdMicros": 12_398_600, "modelId": "gpt-6-sol", "modelObservedAtMs": 1000,
        "pricingVersion": "2026-10-14", "usdMicrosPerMillionInputTokens": 2_000_000,
        "assumption": "uncached-input"});
    document["members"][0]["savings"]["percent"] = serde_json::json!({
        "basisPoints": 1820, "estimatedInputTokensAvoided": 18_200,
        "observedInputTokens": 81_800, "fromMs": 60_000, "throughMs": 120_000});
    document["pricing"] = serde_json::json!({
        "version": "2026-10-14", "asOf": "2026-10-14", "basis": "standard-base-input",
        "models": [
            {"modelId": "claude-opus-5-5", "usdMicrosPerMillionInputTokens": 4_000_000,
             "source": "https://example.test/anthropic-pricing"},
            {"modelId": "gpt-6-sol", "usdMicrosPerMillionInputTokens": 2_000_000,
             "source": "https://example.test/openai-pricing"}]});
    document
}

fn valued_board() -> crate::board::app::App {
    let mut app = board();
    app.labels = crate::labels::Supplied::from_document("digest", &valued_document(), false);
    bind_unit_key(&mut app);
    app
}

/// The action has no default key: a user binds one, as here.
fn bind_unit_key(app: &mut crate::board::app::App) {
    let action = crate::action::Action::parse("savings-unit").unwrap();
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    for table in workspace.keys.values_mut() {
        table.insert("$".into(), action.clone());
    }
}

/// Applies a requested preference write, as the board's event loop does after
/// Config accepts the compare-and-swap.
fn save(app: &mut crate::board::app::App, effect: crate::board::app::Effect) {
    let crate::board::app::Effect::Act(crate::board::app::Request::WorkspacePreferences {
        next,
        ..
    }) = effect
    else {
        panic!("a savings choice must use the existing Config CAS");
    };
    app.finished_workspace_preferences(next, "saved".into());
}

fn click_estimate(app: &mut crate::board::app::App) -> crate::board::app::Effect {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let hits: Vec<_> = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .filter(|(_, choice)| *choice == crate::board::app::Choice::SavingsUnit)
        .map(|(area, _)| *area)
        .collect();
    assert!(!hits.is_empty(), "the painted estimate is clickable");
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hits[0].x,
            row: hits[0].y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    )
}

#[test]
fn a_click_or_the_key_moves_every_surface_to_the_next_unit() {
    use crate::{board::workspace::OpsTarget, config::DigestWindow};
    let _no_commands = crate::runner::forbid_commands();
    for surface in ["member", "squad", "digest", "overview"] {
        let mut app = valued_board();
        match surface {
            "squad" => {
                app.workspace.selected = Some(OpsTarget::Squad(
                    super::super::app::workspace::tests::ROOM.into(),
                ));
                app.workspace_reconcile();
            }
            "digest" => {
                press(&mut app, '3');
            }
            "overview" => app.shell.tab = crate::board::shell::OpsTab::Overview,
            _ => {}
        }
        let shown = paint(&mut app, 160, 36);
        assert!(
            shown.contains("Est. ctx avoided 24h: 18.2k"),
            "{surface}: {shown}"
        );
        // Click: tokens -> percent.
        let effect = click_estimate(&mut app);
        save(&mut app, effect);
        let shown = paint(&mut app, 160, 36);
        assert!(
            shown.contains("Est. ctx avoided 24h: 18% of input"),
            "{surface}: {shown}"
        );
        // Key: percent -> cost, one label and only the value changing.
        let effect = press(&mut app, '$');
        save(&mut app, effect);
        let shown = paint(&mut app, 160, 36);
        assert!(
            shown.contains("Est. ctx avoided 24h: ≈ $12.40 at list price"),
            "{surface}: {shown}"
        );
        assert!(!shown.contains("saved $"), "{surface}");
        // Key: cost -> tokens again.
        let effect = press(&mut app, '$');
        save(&mut app, effect);
        assert!(paint(&mut app, 160, 36).contains("Est. ctx avoided 24h: 18.2k"));
        // Lifetime has no percent: a saved percent reads as tokens and the
        // next step skips to cost.
        let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        workspace.preferences.digest_window = DigestWindow::AllTime;
        workspace.preferences.savings_unit = crate::config::SavingsUnit::Percent;
        assert_eq!(app.savings_unit(), crate::config::SavingsUnit::Tokens);
        let effect = press(&mut app, '$');
        let crate::board::app::Effect::Act(crate::board::app::Request::WorkspacePreferences {
            next,
            ..
        }) = effect
        else {
            panic!("expected a preference write");
        };
        assert_eq!(next.savings_unit, crate::config::SavingsUnit::Cost);
    }
}

#[test]
fn details_carry_the_price_table_and_the_quote_behind_each_figure() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = valued_board();
    press(&mut app, '3');
    let shown = paint(&mut app, 200, 60);
    for expected in [
        "Prices as of 2026-10-14 · table 2026-10-14 · standard-base-input",
        "claude-opus-5-5: $4.00 per 1M input tokens · https://example.test/anthropic-pricing",
        "gpt-6-sol: $2.00 per 1M input tokens",
        "Cost quote: $12.40 at list price · gpt-6-sol at $2.00 per 1M input tokens · price table 2026-10-14",
        "Share of input: 18.20% (18200 avoided of 100000 input tokens)",
    ] {
        assert!(
            shown
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains(expected),
            "{expected}\n{shown}"
        );
    }
    // Without a price table or valuations the details say nothing about them.
    let mut plain = savings_board();
    press(&mut plain, '3');
    let shown = paint(&mut plain, 200, 60);
    assert!(!shown.contains("Prices as of"), "{shown}");
    assert!(!shown.contains("Cost quote"), "{shown}");
}

#[test]
fn the_unit_hint_appears_only_beside_a_painted_estimate() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = valued_board();
    app.shell.tab = crate::board::shell::OpsTab::Overview;
    let shown = paint(&mut app, 160, 44);
    assert!(shown.contains("$ unit"), "{shown}");
    // Narrow enough that the Digest line drops the estimate and no pane paints
    // one: the key still works, but nothing advertises it.
    let mut narrow = valued_board();
    narrow.shell.tab = crate::board::shell::OpsTab::Overview;
    let shown = paint(&mut narrow, 60, 44);
    assert!(!shown.contains("Est."), "{shown}");
    assert!(!shown.contains("$ unit"), "{shown}");
}

#[test]
fn the_action_has_no_default_key_and_the_actions_menu_offers_it_beside_a_figure() {
    use crate::board::app::Choice;
    let _no_commands = crate::runner::forbid_commands();
    for scope in [
        crate::config::OpsKeyScope::Overview,
        crate::config::OpsKeyScope::Squad,
        crate::config::OpsKeyScope::Member,
        crate::config::OpsKeyScope::Pool,
        crate::config::OpsKeyScope::Reader,
    ] {
        assert!(
            crate::config::workspace_defaults(scope)
                .values()
                .all(|action| action.text != "savings-unit"),
            "{scope:?}: an authored binding is the only key"
        );
    }
    let mut app = savings_board();
    paint(&mut app, 160, 36);
    app.context_menu();
    let menu = app.menu.as_ref().expect("the Actions menu opens");
    assert!(
        menu.entries.iter().any(|entry| {
            entry.label == "Savings unit"
                && matches!(&entry.choice, Choice::Action(action) if action.text == "savings-unit")
        }),
        "{:?}",
        menu.entries
            .iter()
            .map(|entry| &entry.label)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_clipped_or_absent_estimate_has_no_hit() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = valued_board();
    press(&mut app, '3');
    paint(&mut app, 160, 36);
    let hit = |app: &crate::board::app::App| {
        app.workspace_strip_hits
            .borrow()
            .iter()
            .filter(|(_, choice)| *choice == crate::board::app::Choice::SavingsUnit)
            .map(|(area, _)| *area)
            .collect::<Vec<_>>()
    };
    let painted = hit(&app);
    assert_eq!(painted.len(), 1);
    // The hit is the caption's own cells, never the whole row.
    assert_eq!(
        painted[0].width as usize,
        "Est. ctx avoided 24h: 18.2k".len()
    );
    // No hit ever extends past the cells that were painted.
    for width in [24u16, 40, 60, 100, 160] {
        let mut narrow = valued_board();
        press(&mut narrow, '3');
        paint(&mut narrow, width, 36);
        for area in hit(&narrow) {
            assert!(area.right() <= width, "width {width}: {area:?}");
            assert_eq!(area.height, 1);
        }
    }
}

#[test]
fn overview_digest_line_shows_the_estimate_and_follows_the_window_without_reading() {
    use crate::config::DigestWindow;
    let _no_commands = crate::runner::forbid_commands();
    let mut app = savings_board();
    app.shell.tab = crate::board::shell::OpsTab::Overview;
    let shown = paint(&mut app, 160, 44);
    assert!(
        shown.contains("Off · Est. ctx avoided 24h: 18.2k"),
        "{shown}"
    );
    // Counts stay measured state; the estimate takes the dim role of every other
    // savings surface.
    let look = app.look();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 44)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let cells = &terminal.backend().buffer().content;
    let symbols: Vec<&str> = cells.iter().map(|cell| cell.symbol()).collect();
    let at = symbols
        .windows(9)
        .position(|window| window == ["O", "f", "f", " ", "·", " ", "E", "s", "t"])
        .expect("the Digest line carries the estimate");
    let has = |cell: &ratatui::buffer::Cell, role| {
        let style: ratatui::style::Style = look.role(role);
        cell.fg == style.fg.unwrap_or(ratatui::style::Color::Reset)
            && cell.modifier.contains(style.add_modifier)
    };
    assert!(has(&cells[at], tmt_cli_style::Role::Muted));
    assert!(has(&cells[at + 6], tmt_cli_style::Role::Dim));
    // The held Overview scene must not outlive the window it summed.
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    workspace.preferences.digest_window = DigestWindow::Week;
    let shown = paint(&mut app, 160, 44);
    assert!(
        shown.contains("Off · Est. ctx avoided 7d: — (0 of 1)"),
        "{shown}"
    );
    // Without Digest evidence the line says so and carries no estimate.
    // A received label set clears the held scene, as the board's event loop does.
    app.overview_rows.borrow_mut().clear();
    app.labels = crate::labels::Supplied::default();
    let shown = paint(&mut app, 160, 44);
    assert!(shown.contains("Digest status unavailable"), "{shown}");
    assert!(!shown.contains("Digest status unavailable · "), "{shown}");
}

#[test]
fn member_savings_remain_visible_at_each_supported_width() {
    for width in [60, 80, 100, 160] {
        let mut app = savings_board();
        app.key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Tab,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        let shown = paint(&mut app, width, 36);
        assert!(shown.contains("Est."), "width {width}: {shown}");
        assert!(shown.contains("18.2k"), "width {width}: {shown}");
    }
}

#[test]
fn digest_all_time_keeps_both_collection_dates_visible_at_narrow_widths() {
    use crate::config::DigestWindow;
    const SECOND: &str = "44444444-4444-4444-8444-444444444444";
    let ms = |text: &str| text.parse::<jiff::Timestamp>().unwrap().as_millisecond() as u64;
    for width in [60, 80, 100, 160] {
        let mut app = savings_board();
        app.local_zone = Some(jiff::tz::TimeZone::get("Asia/Tokyo").unwrap());
        let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        workspace.preferences.digest_window = DigestWindow::AllTime;
        workspace.roster.members.insert(
            SECOND.into(),
            OpsMember {
                id: SECOND.into(),
                name: "second".into(),
                state: None,
                waiting: false,
                presence: None,
                usage: None,
            },
        );
        let mut document = savings_document();
        document["observedAtMs"] = serde_json::json!(1_792_036_800_000u64);
        let mut second = document["members"][0].clone();
        second["identityId"] = serde_json::json!(SECOND);
        document["members"].as_array_mut().unwrap().push(second);
        for (row, start) in document["members"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(["2026-10-01T15:00:00Z", "2026-10-08T15:00:00Z"])
        {
            row["savings"]["window"] = serde_json::json!("lifetime");
            row["savings"]["fromMs"] = serde_json::json!(ms(start));
            row["savings"]["throughMs"] = serde_json::json!(1_792_036_800_000u64);
        }
        app.labels = crate::labels::Supplied::from_document("digest", &document, false);
        press(&mut app, '3');
        let shown = paint(&mut app, width, 36);
        assert!(
            shown.contains("since Digest began recording"),
            "width {width}: {shown}"
        );
        assert!(
            shown.contains("10-02"),
            "first collection date, width {width}: {shown}"
        );
        assert!(
            shown.contains("10-09"),
            "last collection date, width {width}: {shown}"
        );
        assert!(
            shown
                .lines()
                .any(|line| line.contains("members started 10-02 to 10-09")),
            "collection date range stays together, width {width}: {shown}"
        );
    }
}

#[test]
fn window_choice_uses_the_existing_cas_and_never_relabels_old_evidence() {
    use crate::{
        board::app::{Choice, Effect, Request},
        config::DigestWindow,
    };
    let mut app = savings_board();
    let selected = app.workspace.selected.clone();
    app.workspace_settings();
    let menu = app.menu.as_ref().unwrap();
    for window in [DigestWindow::Day, DigestWindow::Week, DigestWindow::AllTime] {
        assert!(
            menu.entries
                .iter()
                .any(|entry| entry.choice == Choice::DigestWindow(window))
        );
    }
    app.menu = None;
    let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) =
        app.choose(Choice::DigestWindow(DigestWindow::Week))
    else {
        panic!("window choice must use the existing Config CAS");
    };
    assert_eq!(expected.digest_window, DigestWindow::Day);
    assert_eq!(next.digest_window, DigestWindow::Week);
    assert_eq!(
        app.digest_window(),
        DigestWindow::Day,
        "pending writes are not reported as saved"
    );
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences = next;
    press(&mut app, '3');
    let shown = paint(&mut app, 160, 36);
    assert!(shown.contains("Est. ctx avoided 7d: — (0 of 1)"), "{shown}");
    assert!(
        !shown.contains("18.2k"),
        "old-window estimate cannot become a new-window value"
    );
    assert_eq!(app.workspace.selected, selected);
}

#[test]
fn estimate_help_keeps_assumptions_and_unavailable_coverage_explicit() {
    let text = crate::board::help::model(&board()).value().to_string();
    assert!(
        text.contains("Estimated context input tokens avoided by delivering messages together.")
    );
    assert!(text.contains("Not billed tokens, not cost, not turns saved."));
    assert!(text.contains("(N of M) counts included members."));
}

/// Explicit renderer evidence only: writes a task-owned packet, never a frozen
/// fixture, and performs no Core, provider, tmux or action-lane acquisition.
#[test]
#[ignore = "explicit Digest savings look packet"]
fn capture_savings_packet() {
    use crate::{
        board::{
            app::workspace::tests::{ROOM, capture_case},
            shell::OpsTab,
        },
        config::DigestWindow,
    };
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    let output = std::env::var("TMT_SAVINGS_CAPTURE").expect("task-owned packet path");
    let _no_commands = crate::runner::forbid_commands();
    let starts = ["2026-10-01T15:00:00Z", "2026-10-08T15:00:00Z"]
        .map(|value| value.parse::<jiff::Timestamp>().unwrap().as_millisecond() as u64);
    let mut captures = Vec::new();
    const SECOND: &str = "44444444-4444-4444-8444-444444444444";
    for scene in ["member", "squad", "digest"] {
        for window in [DigestWindow::Day, DigestWindow::Week, DigestWindow::AllTime] {
            for coverage in ["known", "zero", "partial", "unavailable"] {
                for width in [60, 80, 100, 160] {
                    for (base, depth) in [
                        ("tmt", tmt_cli_style::Depth::TrueColor),
                        ("tmt-light", tmt_cli_style::Depth::TrueColor),
                        ("tmt", tmt_cli_style::Depth::None),
                    ] {
                        let look = crate::look::Look {
                            theme: tmt_cli_style::Theme::new(
                                tmt_cli_style::theme::Base::parse(base).unwrap(),
                            ),
                            depth,
                        };
                        let mut app = capture_case("agents", look);
                        app.local_zone = Some(jiff::tz::TimeZone::get("Asia/Tokyo").unwrap());
                        let workspace =
                            &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                        workspace.preferences.digest_window = window;
                        workspace.roster.members.insert(
                            SECOND.into(),
                            OpsMember {
                                id: SECOND.into(),
                                name: "second".into(),
                                state: Some("working".into()),
                                waiting: false,
                                presence: Some(true),
                                usage: None,
                            },
                        );
                        workspace.roster.squads[0].members.push(SECOND.into());
                        workspace.roster.squads[0].member_states.insert(
                            SECOND.into(),
                            crate::board::workspace::OpsMembershipState {
                                state: Some("working".into()),
                                waiting: false,
                                rank: None,
                            },
                        );
                        workspace.rows.insert(OpsTarget::Member { room_id: Some(ROOM.into()), identity_id: SECOND.into() }, json!({"id":SECOND,"name":"second","state":"working","presence":"active","pane":{"location":{"host":"tmux"}}}));

                        let mut document = savings_document();
                        let mut second = document["members"][0].clone();
                        second["identityId"] = json!(SECOND);
                        second["name"] = json!("second");
                        document["members"].as_array_mut().unwrap().push(second);
                        for (index, row) in document["members"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .enumerate()
                        {
                            let through = 1_792_036_800_000u64;
                            let from = match window {
                                DigestWindow::Day => through - 86_400_000,
                                DigestWindow::Week => through - 604_800_000,
                                DigestWindow::AllTime => starts[index],
                            };
                            let tokens = match coverage {
                                "zero" => json!(0),
                                "unavailable" => json!(null),
                                "partial" if index == 1 => json!(null),
                                _ if index == 0 => json!(18_200),
                                _ => json!(1_400_000),
                            };
                            row["savings"] = json!({"method":"context-input-avoided-v1", "window":window.argument(), "fromMs":from, "throughMs":through, "complete":!tokens.is_null(), "gap":if tokens.is_null() {json!("missing_context")} else {json!(null)}, "estimatedInputTokensAvoided":tokens});
                        }
                        app.labels =
                            crate::labels::Supplied::from_document("digest", &document, false);
                        if scene == "squad" {
                            app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
                        }
                        if scene == "digest" {
                            app.shell.tab = OpsTab::Digest;
                        }
                        app.workspace_reconcile();
                        app.key(ratatui::crossterm::event::KeyEvent::new(
                            ratatui::crossterm::event::KeyCode::Tab,
                            ratatui::crossterm::event::KeyModifiers::NONE,
                        ));
                        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
                        terminal
                            .draw(|frame| {
                                crate::board::view::prepare(&mut app, frame.area());
                                crate::board::view::render(frame, &app);
                            })
                            .unwrap();
                        let buffer = terminal.backend().buffer();
                        let lines = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                            .collect::<Vec<_>>();
                        let cells = buffer
                            .content
                            .iter()
                            .map(|cell| {
                                json!([
                                    cell.symbol(),
                                    format!("{:?}", cell.fg),
                                    format!("{:?}", cell.bg),
                                    format!("{:?}", cell.modifier)
                                ])
                            })
                            .collect::<Vec<_>>();
                        captures.push(json!({"scene":scene,"window":window.argument(),"coverage":coverage,"width":width,"height":36,"base":base,"depth":format!("{depth:?}"),"lines":lines,"cells":cells}));
                    }
                }
            }
        }
    }
    assert_eq!(captures.len(), 432);
    std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
}

#[test]
fn moved_or_hidden_status_keeps_admitted_policy_geometry_bounded_at_every_size() {
    use crate::config::{DigestPanel as Id, DigestPanels, PanelSetting, TokenRate};
    use ratatui::{Terminal, backend::TestBackend};
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut baseline_marks = std::collections::BTreeMap::new();
        for status_position in [None, Some(2), Some(4)] {
            for meter_configured in [false, true] {
                for usage_shown in [false, true] {
                    let mut app = board();
                    app.shell.tab = crate::board::shell::OpsTab::Digest;
                    app.labels = supplied(serde_json::json!(3), false);
                    assert!(
                        !app.loading(),
                        "the policy sweep must use an admitted snapshot"
                    );
                    assert!(crate::tabs::aggregate(app.current.as_deref().unwrap()));
                    let settings = TokenRate {
                        enabled: meter_configured,
                        ..TokenRate::default()
                    };
                    app.meter = Some(crate::board::meter::Meter::new(
                        settings,
                        &crate::board::rate::tests::input(100),
                        std::time::Instant::now(),
                    ));
                    assert_eq!(
                        app.meter.as_ref().unwrap().layout(160).is_some(),
                        meter_configured
                    );
                    let mut policy = DigestPanels::default();
                    if let Some(position) = status_position {
                        for _ in 0..position {
                            policy.move_panel(Id::Status, true);
                        }
                    } else {
                        policy.set_shown(Id::Status, false).unwrap();
                    }
                    policy.set_shown(Id::Usage, usage_shown).unwrap();
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .preferences
                        .digest_panels = Some(PanelSetting::configured(policy));
                    assert!(super::render::status_in_body(&app));
                    for width in 0..=160 {
                        for height in 0..=44 {
                            let mut terminal =
                                Terminal::new(TestBackend::new(width, height)).unwrap();
                            terminal
                                .draw(|frame| {
                                    crate::board::view::prepare(&mut app, frame.area());
                                    crate::board::view::render(frame, &app);
                                })
                                .unwrap();
                            let marks = crate::board::view::time_marks(&app, 1_791_600_000_000);
                            let key = (status_position, usage_shown, width, height);
                            if meter_configured {
                                assert_eq!(
                                    baseline_marks.get(&key),
                                    Some(&marks),
                                    "configured sampling cannot add a named-meter clock"
                                );
                            } else {
                                baseline_marks.insert(key, marks);
                            }
                            let text: String = terminal
                                .backend()
                                .buffer()
                                .content
                                .iter()
                                .map(|cell| cell.symbol())
                                .collect();
                            assert!(
                                !text.contains("no usage reported yet")
                                    && !text.contains("Updating usage…"),
                                "the named meter status is not admitted: {text}"
                            );
                            let area = ratatui::layout::Rect::new(0, 0, width, height);
                            let state = app.conversation.borrow();
                            assert_eq!(state.area.intersection(area), state.area);
                            if let Some(pane) = state.pane {
                                assert_eq!(pane.intersection(area), pane);
                            }
                            for hit in app.hits.borrow().iter() {
                                assert!(
                                    !matches!(hit.target, crate::board::app::HitTarget::Meter(_)),
                                    "an aggregate snapshot cannot grant named-meter authority"
                                );
                                assert!(
                                    hit.x.saturating_add(hit.width) <= width && hit.y < height,
                                    "{status_position:?}/{meter_configured}/{usage_shown}/{width}x{height}: {hit:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    });
}

#[test]
fn hiding_status_does_not_hide_a_failed_observation_or_retain_old_list_controls() {
    use crate::config::{DigestPanel as Id, DigestPanels, PanelSetting};
    let mut app = board();
    app.shell.tab = crate::board::shell::OpsTab::Digest;
    app.labels = supplied(serde_json::json!(3), false);
    paint(&mut app, 160, 36);
    assert!(!app.chip_hits.borrow().is_empty());
    let selected = app.workspace.selected.clone();
    let mut policy = DigestPanels::default();
    policy.set_shown(Id::Status, false).unwrap();
    policy.set_shown(Id::List, false).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .digest_panels = Some(PanelSetting::configured(policy));
    app.labels = Default::default();
    app.invalidate_frame_geometry();
    let text = paint(&mut app, 100, 18);
    assert!(text.contains("Digest status unavailable"), "{text}");
    assert!(app.chip_hits.borrow().is_empty());
    assert_eq!(app.workspace.selected, selected);
    assert!(!app.workspace_selection_visible());
    assert_eq!(app.focused_pane(), Some(crate::config::Pane::Replies));
}

#[test]
fn workspace_status_is_a_body_panel_and_keeps_the_shared_shell_unchanged() {
    use crate::config::{DigestPanel as Id, DigestPanels, PanelSetting};
    use ratatui::{Terminal, backend::TestBackend};
    for width in [60, 100, 160] {
        for position in [0, 1] {
            let mut app = board();
            app.shell.tab = crate::board::shell::OpsTab::Digest;
            app.labels = supplied(serde_json::json!(3), false);
            assert!(!app.loading());
            let expected = super::summary(&app);
            let mut policy = DigestPanels::default();
            for _ in 0..position {
                policy.move_panel(Id::Status, true);
            }
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .preferences
                .digest_panels = Some(PanelSetting::configured(policy));
            let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
            terminal
                .draw(|frame| {
                    crate::board::view::prepare(&mut app, frame.area());
                    crate::board::view::render(frame, &app);
                })
                .unwrap();
            let lines = terminal
                .backend()
                .buffer()
                .content
                .chunks(usize::from(width))
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>();
            let fitted = crate::board::view::fit(&expected, usize::from(width));
            let matching = lines
                .iter()
                .enumerate()
                .filter_map(|(row, line)| (line == &fitted).then_some(row))
                .collect::<Vec<_>>();
            assert_eq!(
                matching.len(),
                1,
                "Status must appear exactly once at {width}: {lines:?}"
            );
            let status_y = matching[0];
            let usage_y = lines
                .iter()
                .position(|line| line.starts_with("Usage · unique identities"))
                .unwrap();
            if position == 0 {
                assert_eq!(status_y + 1, usage_y);
            } else {
                assert!(lines[usage_y + 2].starts_with("Est. ctx avoided"));
                assert_eq!(usage_y + 3, status_y);
            }
            assert!(
                status_y >= 4,
                "Status never replaces the four-row shell chrome"
            );
            assert!(lines[0].contains("OPS / UI"));
            assert!(lines[2].contains("Overview") && lines[2].contains("Digest"));
        }
    }
}
