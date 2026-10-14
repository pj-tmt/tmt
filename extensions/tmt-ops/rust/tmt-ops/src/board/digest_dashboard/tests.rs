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
    assert!(model::summary(&supplied).contains("held unavailable"));
    assert!(!model::summary(&supplied).contains("0 held"));
    let details = model::details(&supplied, MEMBER);
    assert!(details.iter().any(|s| s.contains("inherited default")));
    assert!(details.iter().any(|s| s == "Auto: when idle"));
    assert!(!details.iter().any(|s| s.starts_with("Next send")));
}

#[test]
fn known_zero_and_retained_stale_statistics_stay_distinct() {
    let zero = supplied(serde_json::json!(0), false);
    assert!(model::summary(&zero).contains("0 held"));
    let stale = supplied(serde_json::json!(3), true);
    assert!(model::summary(&stale).contains("3 held · stale observation"));
    assert!(
        model::details(&stale, MEMBER)
            .iter()
            .any(|s| s.contains("Stale observation"))
    );
}

#[test]
fn unavailable_statistics_never_become_zero() {
    assert_eq!(
        model::summary(&Default::default()),
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
    let rows = model::rows(&snapshot, "agent", None);
    assert_eq!(rows.len(), 1);
    assert!(
        matches!(&rows[0].target, OpsTarget::Member { room_id: Some(room), .. } if room == "a")
    );
    assert!(model::rows(&snapshot, "absent", None).is_empty());
    let preferred = OpsTarget::Member {
        room_id: Some("b".into()),
        identity_id: "id".into(),
    };
    assert_eq!(
        model::rows(&snapshot, "agent", Some(&preferred))[0].target,
        preferred
    );
    snapshot.roster.squads[1].members.clear();
    assert_ne!(
        model::rows(&snapshot, "agent", Some(&preferred))[0].target,
        preferred
    );
    snapshot.roster.squads[0].members.clear();
    snapshot.rows.remove(&OpsTarget::Member {
        room_id: None,
        identity_id: "id".into(),
    });
    assert!(
        model::rows(&snapshot, "agent", None).is_empty(),
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
