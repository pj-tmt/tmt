//! WP4 local release-mode evidence uses the existing P02 event/draw profile.
//! Acquisition is absent: a command on the event/paint thread remains forbidden.
use crate::board::{
    app::{App, Effect, workspace::tests::populated_board},
    timing::probe::{measure, report},
    workspace::OpsTarget,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
};
use serde_json::json;

fn labels(id: &str, held: u64) -> crate::labels::Supplied {
    crate::labels::Supplied::from_document(
        "digest",
        &json!({"version":1,"observedAtMs":1_791_600_000_000u64,"members":[{
            "identityId":id,"name":"fixture","mode":"auto","intervalMs":null,"heldCount":held,"statsObservedAtMs":null,"nextSendAtMs":null,"timeToSendMs":null,
            "settingSource":"default","settingValue":"default","labels":[{"text":format!("Auto · {held} held"),"colorClass":"text"}]
        }]}),
        false,
    )
}
fn key(app: &mut App, code: KeyCode) {
    assert!(matches!(
        app.key(KeyEvent::new(code, KeyModifiers::NONE)),
        Effect::None
    ));
}
fn checked(rows: usize, action: &str, samples: Vec<crate::board::timing::probe::Sample>) {
    let p95 = report(rows, action, samples);
    let cap = if rows == 150 { 10_000.0 } else { 50_000.0 };
    assert!(
        p95 <= cap,
        "{rows}/{action} p95 {p95:.1}us exceeds agreed local P02 cap {cap}us"
    );
}
#[test]
#[ignore = "explicit release-mode WP4 P02 evidence; same-machine budget, not CI or user latency"]
fn digest_frame_timing() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    let _no_commands = crate::runner::forbid_commands();
    crate::status::with_now_ms(1_791_600_000_000, || {
        for rows in [150, 1_000, 10_000] {
            let mut app = populated_board(36, rows);
            let squad = &app.workspace_snapshot().unwrap().roster.squads[0];
            let target = OpsTarget::Member {
                room_id: Some(squad.room_id.clone()),
                identity_id: squad.members[0].clone(),
            };
            let id = squad.members[0].clone();
            let index = app
                .workspace_rows()
                .iter()
                .position(|row| row.target == target)
                .unwrap();
            app.select(index);
            app.labels = labels(&id, 3);
            app.view.as_mut().unwrap().home_rate.insert(
                "fixture".into(),
                crate::board::app::RateView {
                    input: crate::board::rate::Input {
                        room: String::new(),
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
            app.limits = Some(
                [("claude", 44.0, Some(97.0)), ("codex", 78.0, None)]
                    .into_iter()
                    .map(|(provider, left, short)| crate::board::limits::Standing {
                        provider: crate::board::limits::Provider::named(provider),
                        latest: Some(crate::board::limits::Reading {
                            observed_at_ms: 1_791_599_940_000,
                            weekly: crate::board::limits::Window {
                                left,
                                resets_at_ms: 1_791_945_600_000,
                            },
                            short: short.map(|left| crate::board::limits::Window {
                                left,
                                resets_at_ms: 1_791_610_800_000,
                            }),
                        }),
                        burn: Some(0.4),
                        trend: None,
                    })
                    .collect(),
            );
            key(&mut app, KeyCode::Char('2'));
            let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
            for _ in 0..10 {
                measure(&mut app, &mut terminal, |_, _| {});
            }
            checked(
                rows,
                "digest-steady",
                (0..100)
                    .map(|_| measure(&mut app, &mut terminal, |_, _| {}))
                    .collect(),
            );
            let updates = [labels(&id, 3), labels(&id, 4)];
            checked(
                rows,
                "digest-status-projection",
                (0..100)
                    .map(|i| {
                        measure(&mut app, &mut terminal, |app, _| {
                            app.labels = updates[i % 2].clone()
                        })
                    })
                    .collect(),
            );
            checked(
                rows,
                "digest-tab-switch",
                (0..100)
                    .map(|i| {
                        measure(&mut app, &mut terminal, |app, _| {
                            key(app, KeyCode::Char(if i % 2 == 0 { '1' } else { '2' }))
                        })
                    })
                    .collect(),
            );
            key(&mut app, KeyCode::Char('2'));
            app.focus = 0;
            checked(
                rows,
                "digest-list-scroll",
                (0..100)
                    .map(|i| {
                        measure(&mut app, &mut terminal, |app, _| {
                            key(
                                app,
                                if i % 2 == 0 {
                                    KeyCode::Down
                                } else {
                                    KeyCode::Up
                                },
                            )
                        })
                    })
                    .collect(),
            );
            checked(
                rows,
                "digest-resize",
                (0..100)
                    .map(|i| {
                        measure(&mut app, &mut terminal, |app, terminal| {
                            app.invalidate_frame_geometry();
                            let width = if i % 2 == 0 { 100 } else { 160 };
                            terminal.backend_mut().resize(width, 36);
                            terminal.resize(Rect::new(0, 0, width, 36)).unwrap();
                        })
                    })
                    .collect(),
            );
            // The shared editor remains responsive while the supplier is unavailable.
            app.labels = Default::default();
            key(&mut app, KeyCode::Char('1'));
            key(&mut app, KeyCode::Enter);
            assert!(app.input.is_some());
            for kib in [1, 10, 100] {
                let text = "x".repeat(kib * 1024);
                let mut samples = Vec::new();
                for _ in 0..100 {
                    app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
                    app.key(KeyEvent::new(
                        KeyCode::End,
                        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                    ));
                    key(&mut app, KeyCode::Backspace);
                    measure(&mut app, &mut terminal, |_, _| {});
                    samples.push(measure(&mut app, &mut terminal, |app, _| {
                        app.paste(&text);
                    }));
                    assert_eq!(app.input.as_ref().unwrap().text, text.as_str());
                }
                checked(rows, &format!("digest-unavailable-paste-{kib}KiB"), samples);
            }
            assert_eq!(app.workspace.selected, Some(target));
        }
    });
}

fn full_labels(app: &App, changed_id: &str, held: u64) -> crate::labels::Supplied {
    let members = app.workspace_snapshot().unwrap().roster.members.values().map(|member| {
        let held=if member.id==changed_id {held}else{0};
        json!({"identityId":member.id,"name":member.name,"mode":"auto","intervalMs":null,
            "heldCount":held,"statsObservedAtMs":1_791_600_000_000u64,"nextSendAtMs":null,"timeToSendMs":null,
            "settingSource":"default","settingValue":"default","labels":[{"text":format!("Auto · {held} held"),"colorClass":"text","action":{
                "kind":"choose","argv":["digest",member.id,"{value}"],"options":[{"label":"Default (Auto)","value":"default"},{"label":"Auto","value":"auto"},{"label":"Off","value":"off"}],"current":"default"
            }}]})
    }).collect::<Vec<_>>();
    crate::labels::Supplied::from_document(
        "digest",
        &json!({"version":1,"observedAtMs":1_791_600_000_000u64,"members":members}),
        false,
    )
}

#[test]
#[ignore = "explicit release-mode full supplier-snapshot WP4 P02 evidence"]
fn digest_full_status_frame_timing() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    crate::status::with_now_ms(1_791_600_000_000, || {
        for rows in [150, 1_000, 10_000] {
            let mut app = populated_board(36, rows);
            let squad = &app.workspace_snapshot().unwrap().roster.squads[0];
            let target = OpsTarget::Member {
                room_id: Some(squad.room_id.clone()),
                identity_id: squad.members[0].clone(),
            };
            let id = squad.members[0].clone();
            let index = app
                .workspace_rows()
                .iter()
                .position(|row| row.target == target)
                .unwrap();
            app.select(index);
            let updates = [full_labels(&app, &id, 3), full_labels(&app, &id, 4)];
            app.labels = updates[0].clone();
            key(&mut app, KeyCode::Char('2'));
            let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
            for _ in 0..10 {
                measure(&mut app, &mut terminal, |_, _| {});
            }
            checked(
                rows,
                "digest-full-status-steady",
                (0..100)
                    .map(|_| measure(&mut app, &mut terminal, |_, _| {}))
                    .collect(),
            );
            let mut samples = Vec::new();
            for i in 0..100 {
                // Acquisition/fixture cloning is off the input path. The actual
                // event compares, replaces and drops the previous owned snapshot.
                let supplied = updates[(i + 1) % 2].clone();
                samples.push(measure(&mut app, &mut terminal, |app, _| {
                    assert!(app.labels != supplied);
                    app.labels = supplied;
                }));
            }
            checked(rows, "digest-full-status-replacement", samples);
            assert_eq!(app.workspace.selected, Some(target));
        }
    });
}
