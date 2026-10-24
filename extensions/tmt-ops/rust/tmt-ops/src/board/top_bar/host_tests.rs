use crate::board::{
    app::workspace::tests::populated_board,
    inventory::Inventory,
    limits::{Provider, Reading, Standing, Window},
    shell::OpsTab,
    view::{self, ClockSurface},
    workspace::OpsTarget,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

pub(super) fn observed_app(now: u64) -> (crate::board::app::App, Vec<String>, Vec<String>) {
    let mut app = populated_board(1, 2);
    app.workspace.expanded.insert(
        app.workspace_snapshot().unwrap().roster.squads[0]
            .room_id
            .clone(),
    );
    app.view.as_mut().unwrap().home_rate.insert(
        "product".into(),
        crate::board::app::RateView {
            input: crate::board::rate::Input {
                room: crate::board::app::workspace::tests::ROOM.into(),
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
    let members = app.workspace_snapshot().unwrap().roster.squads[0]
        .members
        .clone();
    let names: Vec<_> = members
        .iter()
        .map(|id| {
            app.workspace_snapshot().unwrap().roster.members[id]
                .name
                .clone()
        })
        .collect();
    app.references = Inventory::from_listed(
        &[],
        &Ok(json!({"identities": [
            {"id":members[0], "name":names[0], "runningDriver":"codex",
             "resume":{"usage":{"tokens":142000,"windowTokens":258000,"observedAtMs":now}}},
            {"id":members[1], "name":names[1], "runningDriver":"claude",
             "resume":{"usage":{"tokens":0,"observedAtMs":now}}}
        ]})),
    );
    app.limits = Some(vec![
        Standing {
            rates: Default::default(),
            provider: Provider::named("claude"),
            latest: Some(Reading {
                observed_at_ms: now,
                weekly: Window {
                    left: 41.0,
                    resets_at_ms: now + 10000,
                },
                short: Some(Window {
                    left: 62.0,
                    resets_at_ms: now + 5000,
                }),
            }),
            burn: None,
            trend: None,
        },
        Standing {
            rates: Default::default(),
            provider: Provider::named("codex"),
            latest: Some(Reading {
                observed_at_ms: now,
                weekly: Window {
                    left: 14.0,
                    resets_at_ms: now + 10000,
                },
                short: None,
            }),
            burn: None,
            trend: None,
        },
    ]);
    (app, members, names)
}

#[test]
fn every_tab_uses_the_selected_uuid_and_held_limits_without_commands() {
    let now = 1_791_600_000_000;
    crate::status::with_now_ms(now, || {
        let _no_commands = crate::runner::forbid_commands();
        let (mut app, members, names) = observed_app(now);
        let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
        for tab in [OpsTab::Agents, OpsTab::Digest] {
            app.shell.tab = tab;
            for (id, name, expected) in [
                (&members[0], &names[0], "142k / 258k"),
                (&members[1], &names[1], "0"),
            ] {
                app.workspace.selected = Some(OpsTarget::Member {
                    room_id: Some(
                        app.workspace_snapshot().unwrap().roster.squads[0]
                            .room_id
                            .clone(),
                    ),
                    identity_id: id.clone(),
                });
                terminal.draw(|frame| view::render(frame, &app)).unwrap();
                let text: String = terminal.backend().buffer().content[..160]
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(text.contains("Claude 5h 62% · 7d 41%"), "{tab:?}: {text}");
                assert!(
                    text.contains(&format!("│ {name} ctx {expected}")),
                    "{tab:?}: {text}"
                );
                assert!(!text.contains("wk"));
            }
        }
        app.frame_clocks
            .borrow_mut()
            .retain(|surface| matches!(surface, ClockSurface::TopBar { .. }));
        assert_eq!(app.frame_clocks.borrow().len(), 1);
        let before = view::time_marks(&app, now);
        let after = view::time_marks(&app, now + 10001);
        assert_ne!(
            before, after,
            "expired limits must invalidate the actual painted top bar"
        );
        assert_eq!(after, view::time_marks(&app, now + 10002));
        app.workspace.selected = Some(OpsTarget::Member {
            room_id: None,
            identity_id: members[0].clone(),
        });
        terminal.draw(|frame| view::render(frame, &app)).unwrap();
        let invalid: String = terminal.backend().buffer().content[..160]
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            !invalid.contains(" ctx "),
            "a member outside the selected room is not an admitted selected target"
        );
        app.workspace.selected = Some(OpsTarget::Pool);
        terminal.draw(|frame| view::render(frame, &app)).unwrap();
        let text: String = terminal.backend().buffer().content[..160]
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            !text.contains(" ctx "),
            "a non-member must not inherit the previous member's context"
        );
    });
}

#[test]
fn shrinking_drops_whole_segments_in_the_approved_order() {
    use super::{Attention, Observations, paint};
    use crate::{board::inventory::ContextUsage, look::Look};
    use ratatui::{buffer::Buffer, layout::Rect};
    let now = 10_000;
    let limits = [
        Standing {
            rates: Default::default(),
            provider: Provider::named("claude"),
            latest: Some(Reading {
                observed_at_ms: now,
                weekly: Window {
                    left: 41.0,
                    resets_at_ms: now + 10000,
                },
                short: Some(Window {
                    left: 62.0,
                    resets_at_ms: now + 5000,
                }),
            }),
            burn: None,
            trend: None,
        },
        Standing {
            rates: Default::default(),
            provider: Provider::named("codex"),
            latest: Some(Reading {
                observed_at_ms: now,
                weekly: Window {
                    left: 14.0,
                    resets_at_ms: now + 10000,
                },
                short: None,
            }),
            burn: None,
            trend: None,
        },
    ];
    let held = Observations {
        limits: &limits,
        context: Some((
            "Ben",
            ContextUsage {
                used_tokens: 142000,
                window_tokens: Some(258000),
                observed_at_ms: now,
            },
        )),
        now,
    };
    let attention = Attention {
        online: "28".into(),
        unread: Default::default(),
        todo: "9 todo".into(),
        blocked: "1".into(),
        todo_positive: true,
        blocked_positive: true,
        complete: true,
        ascii: false,
    };
    for (width, weekly, words, window, brand) in [
        (130, true, true, true, true),
        (100, false, true, true, true),
        (80, false, false, true, true),
        (68, false, false, false, true),
        (60, false, false, false, false),
    ] {
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        paint(
            &mut buffer,
            area,
            None,
            Some(&attention),
            Some(&held),
            Look::default(),
        );
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert_eq!(text.contains("7d"), weekly, "{width}: {text}");
        assert_eq!(text.contains("online"), words, "{width}: {text}");
        assert_eq!(text.contains(" / 258k"), window, "{width}: {text}");
        assert_eq!(text.contains("OPS / UI"), brand, "{width}: {text}");
        for value in ["Claude 5h 62%", "│ Ben ctx 142k", "28", "9", "1"] {
            assert!(text.contains(value), "{width}: missing {value}: {text}");
        }
    }
}

#[test]
fn the_last_fit_stage_ellipsizes_the_name_without_discarding_known_values() {
    use super::{Observations, paint};
    use crate::{board::inventory::ContextUsage, look::Look};
    use ratatui::{buffer::Buffer, layout::Rect};
    let (app, _, _) = observed_app(1_791_600_000_000);
    let held = Observations {
        limits: app.limits.as_deref().unwrap(),
        context: Some((
            "agent-with-a-long-name",
            ContextUsage {
                used_tokens: 142000,
                window_tokens: Some(258000),
                observed_at_ms: 1,
            },
        )),
        now: 1_791_600_000_000,
    };
    let attention = super::Attention {
        online: "28".into(),
        unread: Default::default(),
        todo: "9 todo".into(),
        blocked: "1".into(),
        todo_positive: true,
        blocked_positive: true,
        complete: true,
        ascii: false,
    };
    let area = Rect::new(0, 0, 60, 1);
    let mut buffer = Buffer::empty(area);
    paint(
        &mut buffer,
        area,
        None,
        Some(&attention),
        Some(&held),
        Look::default(),
    );
    let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains("Claude 5h 62%"), "{text}");
    assert!(
        text.starts_with(" ▚ TMT  Claude"),
        "normal spacing after the dropped caption: {text}"
    );
    assert!(
        text.contains("agent-wit… ctx 142k"),
        "freed caption cells belong to the member name: {text}"
    );
    assert!(
        !text.contains(" / 258k") && !text.contains("7d") && !text.contains("OPS / UI"),
        "{text}"
    );
}
