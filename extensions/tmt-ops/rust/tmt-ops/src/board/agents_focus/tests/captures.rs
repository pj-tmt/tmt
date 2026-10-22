//! Literal acquired observations for the approved member/squad geometry.
use super::*;
use crate::board::{
    app::{RateView, tests::install_rate_observation_at},
    inventory::{Agent, Inventory},
    rate::{
        Input,
        history::{Seed, Slice},
    },
    workspace::{OpsMember, OpsMembershipState, OpsObservation, OpsRoster, OpsSquad},
};
use std::collections::BTreeMap;
const NOW: u64 = 1_791_600_000_000;
const HOUR: u64 = 3_600_000;

fn uuid(n: usize) -> String {
    format!("44444444-4444-4444-8444-{n:012}")
}

fn fixture(selected_name: &str, look: Look) -> App {
    let mut app = observed("Observed reply retained for the selected actor and recipient.");
    type Member = (&'static str, &'static str, Option<u64>);
    let groups: [(&str, &[Member]); 8] = [
        (
            "mamezu",
            &[
                ("mamezu-1", "working", None),
                ("mamezu-2", "working", None),
                ("mamezu-3", "working", None),
            ],
        ),
        (
            "tmt-colab",
            &[
                ("colab-lead", "waiting", None),
                ("colab-1", "idle", None),
                ("colab-2", "idle", None),
            ],
        ),
        (
            "tmt-core",
            &[
                ("tmt-lead", "working", Some(6_100_000)),
                ("core-1", "idle", Some(0)),
                ("core-ops", "working", Some(0)),
                ("tmt-core-pm", "working", Some(2_100_000)),
            ],
        ),
        (
            "tmt-infra",
            &[
                ("infra-lead", "waiting", Some(1_600_000)),
                ("infra-1", "waiting", Some(1_500_000)),
            ],
        ),
        (
            "tmt-mkt",
            &[
                ("mkt-lead", "waiting", Some(400_000)),
                ("mkt-1", "waiting", Some(0)),
                ("mkt-2", "idle", Some(0)),
            ],
        ),
        (
            "tmt-night",
            &[
                ("night-lead", "working", Some(5_600_000)),
                ("night-1", "working", Some(0)),
                ("night-2", "waiting", Some(15_100_000)),
                ("night-3", "waiting", Some(5_000_000)),
                ("night-muse", "working", None),
            ],
        ),
        (
            "tmt-remote",
            &[
                ("remote-lead", "working", Some(10_000_000)),
                ("remote-2", "waiting", Some(11_900_000)),
            ],
        ),
        (
            "tmt-ux",
            &[
                ("tmt-ux-lead", "waiting", Some(18_900_000)),
                ("ux-1", "idle", Some(0)),
                ("ux-revamp", "working", Some(31_200_000)),
            ],
        ),
    ];
    let mut roster = OpsRoster {
        observation: OpsObservation::Complete,
        membership_complete: true,
        ..Default::default()
    };
    let mut rows = BTreeMap::from([(OpsTarget::Pool, json!({"name":"Common pool","kind":"pool"}))]);
    let mut inventory = Inventory::default();
    let mut selected = None;
    let mut rates = Vec::new();
    let mut next = 1;
    for (ordinal, (name, members)) in groups.into_iter().enumerate() {
        let room = uuid(100 + ordinal);
        let mut ids = Vec::new();
        let mut states = BTreeMap::new();
        let mut resumes = BTreeMap::new();
        let mut seeds = BTreeMap::new();
        for (name, state, tokens) in members {
            let id = uuid(next);
            next += 1;
            let target = OpsTarget::Member {
                room_id: Some(room.clone()),
                identity_id: id.clone(),
            };
            if *name == selected_name {
                selected = Some(target.clone());
            }
            ids.push(id.clone());
            states.insert(
                id.clone(),
                OpsMembershipState {
                    state: Some((*state).into()),
                    waiting: *state == "waiting",
                    rank: None,
                    idle_over: false,
                },
            );
            roster.members.insert(
                id.clone(),
                OpsMember {
                    id: id.clone(),
                    name: (*name).into(),
                    state: Some((*state).into()),
                    waiting: *state == "waiting",
                    presence: Some(next <= 24),
                    usage: None,
                    // Provider observations are independent from the task-state seeds.
                    activity: crate::board::activity::ObservedActivity {
                        phase: match *name {
                            "colab-1" | "colab-2" | "core-1" | "mkt-2" | "ux-1" => {
                                crate::board::activity::Phase::Idle
                            }
                            "night-muse" => crate::board::activity::Phase::Unknown,
                            _ => crate::board::activity::Phase::Working,
                        },
                        ..Default::default()
                    },
                },
            );
            rows.insert(target, json!({"id":id,"name":name,"state":state,"presence":"active","pane":{"location":{"host":"tmux"}},
                "fields":{"model":"gpt-6.1-sol","task":"Read-only UI audit", "pending":"", "next":"Review selected observations"}}));
            inventory.agents.insert(
                id.clone(),
                Agent {
                    parameters: json!({}),
                    runtime: Some("codex".into()),
                    model: Some("gpt-6.1-sol".into()),
                    name: (*name).into(),
                    canonical: (*name).into(),
                    basic: false,
                    context: None,
                },
            );
            resumes.insert(id.clone(), json!({}));
            if let Some(total) = tokens {
                let through = NOW;
                let from = through - HOUR;
                let buckets = (0..120)
                    .map(|bucket| Slice {
                        from_ms: from + bucket * 30_000,
                        to_ms: from + (bucket + 1) * 30_000,
                        input_tokens: total / 120 + u64::from(bucket < total % 120),
                        output_tokens: 0,
                        cached_input_tokens: 0,
                        covered_ms: 30_000,
                        complete: true,
                        gap: false,
                        discontinuous: false,
                        by_driver: Vec::new(),
                    })
                    .collect();
                seeds.insert(
                    id.clone(),
                    Some(Seed {
                        from,
                        through,
                        available: Some(from),
                        sampled: Some(through - 1),
                        latest: json!({}),
                        reporting: true,
                        buckets,
                    }),
                );
            } else {
                seeds.insert(id.clone(), None);
            }
        }
        let squad_target = OpsTarget::Squad(room.clone());
        if name == selected_name {
            selected = Some(squad_target.clone());
        }
        rows.insert(
            squad_target,
            json!({"name":name,"roomId":room,"memberIds":ids,"revision":1}),
        );
        roster.squads.push(OpsSquad {
            room_id: room.clone(),
            name: name.into(),
            lead: ids.first().cloned(),
            members: ids,
            member_states: states,
            observation: OpsObservation::Complete,
            idle_alert: None,
        });
        rates.push((
            name.to_owned(),
            RateView {
                input: Input {
                    room,
                    resumes,
                    names: Default::default(),
                },
                settings: crate::config::TokenRate {
                    enabled: true,
                    window: crate::config::TokenWindow::HOUR,
                    ..Default::default()
                },
                history: Some(seeds),
            },
        ));
    }
    for ordinal in 0..3 {
        let id = uuid(200 + ordinal);
        roster.members.insert(
            id.clone(),
            OpsMember {
                id,
                name: format!("pool-{ordinal}"),
                state: None,
                waiting: false,
                presence: Some(false),
                usage: None,
                activity: Default::default(),
            },
        );
    }
    let view = app.view.as_mut().unwrap();
    view.look = look;
    let snapshot = &mut view.home.as_mut().unwrap().workspace;
    snapshot.roster = roster;
    snapshot.rows = rows;
    snapshot.squad_looks.clear();
    snapshot.shell_counts = snapshot.roster.shell_counts();
    app.references = inventory;
    for (name, rate) in rates {
        install_rate_observation_at(
            &mut app,
            name,
            rate,
            std::time::Instant::now() + std::time::Duration::from_secs(60),
        );
    }
    let labels = app
        .references
        .agents
        .iter()
        .map(|(id, agent)| {
            let squad = app
                .workspace_snapshot()
                .unwrap()
                .roster
                .squads
                .iter()
                .find(|squad| squad.members.contains(id))
                .unwrap();
            json!({"identityId":id,"labels":[{"text":"Auto","colorClass":"text",
            "action":{"kind":"choose","options":[{"label":"Auto","value":"auto"}],
            "current":"auto","argv":["digest",agent.name,"--squad",squad.name,"{value}"]}},
            {"text":"held 0 · ctx —","colorClass":"muted"}]})
        })
        .collect::<Vec<_>>();
    app.labels = crate::labels::Supplied::from_document(
        "digest",
        &json!({"version":1,"members":labels}),
        false,
    );
    app.workspace.selected = selected;
    let workspace = app.workspace_snapshot().unwrap();
    let expanded = workspace
        .roster
        .squads
        .iter()
        .filter(|squad| ["tmt-core", "tmt-night", "tmt-ux"].contains(&squad.name.as_str()))
        .map(|squad| squad.room_id.clone())
        .collect();
    app.workspace.expanded = expanded;
    app.workspace_reconcile();
    let target = app.workspace_message_target().unwrap();
    let id = match target {
        OpsTarget::Member { identity_id, .. } => identity_id,
        _ => unreachable!(),
    };
    app.view.as_mut().unwrap().replies = vec![
        json!({"recipientId":id,"requestId":"actor-reply","submittedAtMs":NOW - 720_000,"response": if selected_name == "core-1" {"Will do once the current PR lands."} else {"Read-only until 09:30; findings stay on the page."}}),
    ];
    let actor = app.view.as_ref().unwrap().me_id.clone().unwrap();
    let mut items = Vec::new();
    for (name, count) in [("core-ops", 1), ("night-1", 2)] {
        let id = app
            .references
            .agents
            .iter()
            .find(|(_, agent)| agent.name == name)
            .unwrap()
            .0;
        for n in 0..count {
            items.push(json!({"requestId":format!("read-{name}-{n}"),"kind":"request","sender":{"identityId":actor},"recipientId":id,"recipientAcknowledged":true,"final":{"status":"retained","submittedAtMs":NOW - 1000 + n}}));
        }
    }
    app.home_leads
        .test_results(&actor, json!({"items":items,"nextBefore":null}));
    app.cron.replace(Ok(crate::board::cronboard::test_cron(
        vec![],
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    app
}

#[test]
fn approved_focus_geometry_has_cell_captures_at_both_widths_and_all_looks() {
    crate::status::with_now_ms(NOW, || {
        let mut frames = Vec::new();
        for (name, base, depth) in [
            (
                "tmt",
                tmt_cli_style::Base::Tmt,
                tmt_cli_style::Depth::TrueColor,
            ),
            (
                "tmt-light",
                tmt_cli_style::Base::TmtLight,
                tmt_cli_style::Depth::TrueColor,
            ),
            (
                "terminal",
                tmt_cli_style::Base::Terminal,
                tmt_cli_style::Depth::Ansi16,
            ),
            (
                "NO_COLOR",
                tmt_cli_style::Base::Tmt,
                tmt_cli_style::Depth::None,
            ),
        ] {
            for selected in ["night-2", "core-1", "tmt-night"] {
                for width in [103, 148] {
                    let mut app = fixture(
                        selected,
                        Look {
                            theme: tmt_cli_style::Theme::new(base),
                            depth,
                        },
                    );
                    let buffer = draw(&mut app, width, 51);
                    let text = screen(&buffer);
                    assert!(
                        text.contains("8 squads · 28 agents"),
                        "{selected}/{width}: {text}"
                    );
                    assert!(text.contains("Exchanges with you"));
                    if selected == "tmt-night" {
                        let pane_text = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| {
                                row[if width == 148 { 71 } else { 0 }..]
                                    .iter()
                                    .map(|cell| cell.symbol())
                                    .collect::<String>()
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        // The squad's own Usage section is the last one; the band above also says Usage.
                        let usage = pane_text.rsplit("Usage").next().unwrap();
                        assert!(
                            usage.find("night-3 ").unwrap() < usage.find("night-muse ").unwrap()
                        );
                        assert!(
                            usage.find("night-muse ").unwrap() < usage.find("night-1 ").unwrap()
                        );
                    }
                    assert!(
                        !text.contains("all parties")
                            && !text.contains("since ")
                            && !text.contains("$link")
                    );
                    let pane = app.conversation.borrow().pane.unwrap();
                    if width == 148 {
                        assert_eq!((pane.x, pane.y, pane.width), (71, 9, 77));
                    } else {
                        assert_eq!(pane.x, 0);
                        assert!(pane.y > 9);
                    }
                    let band = app.input_band.get().unwrap();
                    assert_eq!(band.height, 4);
                    assert_eq!(band.bottom(), pane.bottom());
                    assert!(text.contains(if selected == "tmt-night" {
                        "lead night-lead"
                    } else {
                        selected
                    }));
                    frames.push(json!({"scene":selected,"look":name,"width":width,"height":51,
                        "lines":text.split('\n').collect::<Vec<_>>(),
                        "cells":buffer.content.iter().map(|cell| json!([cell.symbol(),format!("{:?}",cell.fg),format!("{:?}",cell.bg),format!("{:?}",cell.modifier)])).collect::<Vec<_>>(),
                        "input":format!("{band:?}"),"hits":format!("{:?}", app.hits.borrow()),"actions":format!("{:?}",app.workspace_action_hits.borrow())}));
                }
            }
        }
        if let Some(path) = std::env::var_os("TMT_AGENTS_FOCUS_CAPTURE") {
            std::fs::write(path, serde_json::to_vec(&json!({"source":"approved Colab 739017fe; current actor-only observations, no waiting ages or inline keys", "frames":frames})).unwrap()).unwrap();
        }
    });
}
