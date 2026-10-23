//! Six sent-message states through the real composer and held observation ports.
//! The frozen oracle and task-owned mock-width captures use the same fixtures.
use super::*;
use crate::board::{activity::Phase, app::workspace::tests::MEMBER};
use crate::look::Look;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) const SCENES: [&str; 15] = [
    "sent-sending",
    "sent-working",
    "sent-earlier",
    "sent-replied",
    "sent-finished",
    "sent-unknown",
    "chat-pending",
    "chat-bound",
    "chat-answered",
    "chat-no-answer",
    "chat-truncated",
    "chat-fallback",
    "chat-reconciled",
    "chat-reconciled-restored",
    "chat-unbound",
];
const QUESTION: &str = "Can you rerun the picker tests on main?";
const REPLY: &str = "Yes. All 14 picker tests pass on main 6899a9f2.";
const REQUEST: &str = "req_55555555-5555-4555-8555-555555555555";
const UNBOUND_REASON: &str = "session not bound · run: tmt resume -- night-2";

pub(in crate::board) fn fixture(scene: &str, look: Look, now_ms: u64) -> App {
    assert!(SCENES.contains(&scene));
    if matches!(scene, "chat-reconciled" | "chat-reconciled-restored") {
        let app = window_capture_fixture(scene.ends_with("-restored"), look, now_ms);
        observe_pane(&app, vec![window_item(now_ms)]);
        return app;
    }
    if scene.starts_with("chat-") {
        return chat_fixture(scene, look, now_ms);
    }
    let mut app = crate::board::app::workspace::tests::board();
    let target = app.workspace.selected.clone().unwrap();
    let name = if scene == "sent-unknown" {
        "mkt-gemini"
    } else {
        "night-2"
    };
    let view = app.view.as_mut().unwrap();
    view.look = look;
    view.me = Some("you".into());
    let home = view.home.as_mut().unwrap();
    home.workspace.squad_looks.clear();
    home.workspace.rows.get_mut(&target).unwrap()["name"] = json!(name);
    home.workspace.roster.members.get_mut(MEMBER).unwrap().name = name.into();
    let sent = match scene {
        "sent-sending" => now_ms - 1_000,
        "sent-earlier" | "sent-working" => now_ms - 42_000,
        _ => now_ms - 180_000,
    };
    let (ticket, _) = crate::status::with_now_ms(sent, || begin(&mut app, QUESTION));
    if scene != "sent-sending" {
        finish(
            &mut app,
            Completed {
                ticket: Some(ticket),
                outcome: Ok(Accepted {
                    request: REQUEST.into(),
                    disposition: Disposition::Sent,
                    passthrough: false,
                    message_state_label: None,
                    can_send_now: false,
                    working: false,
                    fallback: None,
                    warning: None,
                }),
                what: "Message".into(),
                to: name.into(),
            },
        );
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Effect::None
        );
        assert!(
            app.input.is_none(),
            "accepted-send captures use the mock's idle input"
        );
        // Omit the transient empty-editor-close toast from durable-state frames.
        app.notice = None;
    }
    app.focus = 1;
    match scene {
        "sent-working" => provider_observation(&mut app, Phase::Working, now_ms - 42_000),
        "sent-earlier" => provider_observation(&mut app, Phase::Working, now_ms - 180_000),
        "sent-replied" => app.view.as_mut().unwrap().replies.push(json!({
            "recipientId":MEMBER,"requestId":REQUEST,"submittedAtMs":now_ms,"response":REPLY
        })),
        "sent-finished" => {
            provider_observation(&mut app, Phase::Working, sent);
            crate::status::with_now_ms(sent, || board_frame(&app, 148, 51));
            provider_observation(&mut app, Phase::Idle, now_ms - 120_000);
        }
        _ => {
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .roster
                .members
                .get_mut(MEMBER)
                .unwrap()
                .activity = Default::default()
        }
    }
    app
}

fn chat_fixture(scene: &str, look: Look, now_ms: u64) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    let target = app.workspace.selected.clone().unwrap();
    let view = app.view.as_mut().unwrap();
    view.look = look;
    view.me = Some("you".into());
    let home = view.home.as_mut().unwrap();
    home.workspace.squad_looks.clear();
    home.workspace.rows.get_mut(&target).unwrap()["name"] = json!("night-2");
    home.workspace.roster.members.get_mut(MEMBER).unwrap().name = "night-2".into();
    let at = now_ms
        - if scene == "chat-bound" {
            3_600_000
        } else {
            42_000
        };
    let (ticket, _) = crate::status::with_now_ms(at, || begin(&mut app, QUESTION));
    let sender = ticket.key.0.clone();
    let fallback = scene == "chat-fallback";
    finish(
        &mut app,
        Completed {
            ticket: Some(ticket),
            outcome: Ok(Accepted {
                request: REQUEST.into(),
                disposition: if fallback {
                    Disposition::Queued
                } else {
                    Disposition::Sent
                },
                passthrough: !fallback,
                message_state_label: Some(
                    if fallback {
                        "✗ not sent, the agent could not be reached"
                    } else if scene == "chat-bound" {
                        "⠹ the agent has started on it"
                    } else if scene == "chat-unbound" {
                        "✓ session not bound · run: tmt resume -- night-2"
                    } else {
                        "✓ delivered"
                    }
                    .into(),
                ),
                can_send_now: false,
                working: scene == "chat-bound",
                fallback: fallback.then(|| "There is no live pane.".into()),
                warning: None,
            }),
            what: "Message".into(),
            to: "night-2".into(),
        },
    );
    // Selected-read admission follows the same visible-pane lease as production.
    crate::status::with_now_ms(now_ms, || board_frame(&app, 148, 51));
    if !fallback {
        assert!(app.conversation_finals().is_some(), "visible capture owner");
        let state = match scene {
            "chat-pending" | "chat-unbound" => "pending",
            "chat-bound" => "bound",
            "chat-no-answer" => "no_answer",
            _ => "answered",
        };
        app.apply_sent(crate::board::home_leads::SentKey { sender, agent: MEMBER.into() }, Ok(vec![json!({
            "requestId":REQUEST,"preparedAtMs":at,"text":QUESTION,"truncated":false,"mode":"passthrough",
            "messageState":{"state":if scene=="chat-bound" {"working"} else {"delivered"},"reason":{"text":if scene=="chat-bound" {"the agent has started on it"} else if scene=="chat-unbound" {UNBOUND_REASON} else {"delivered"}}},
            "passthrough":{"state":state,"boundAtMs":at+1,"answer":{"text":REPLY,"truncated":scene=="chat-truncated","provider":"codex","endedMs":now_ms-1000}}
        })]));
    }
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.notice = None;
    app.focus = 1;
    app
}

pub(in crate::board) fn expected(scene: &str) -> &'static str {
    match scene {
        "sent-sending" => "sending…",
        "sent-working" => "night-2 is working · 0:42",
        "sent-earlier" => "◌ night-2 is finishing an earlier turn · 3m",
        "sent-replied" => REPLY,
        "sent-finished" => "✓ night-2 finished without replying · 2m ago",
        "sent-unknown" => "✓ sent · mkt-gemini's activity is not reported",
        "chat-pending" => "delivered",
        "chat-bound" => "the agent has started on it",
        "chat-unbound" => "session not bound",
        "chat-answered" | "chat-reconciled" | "chat-reconciled-restored" => "turn output",
        "chat-truncated" => "turn output · truncated",
        "chat-no-answer" => "No turn output was captured for this message.",
        "chat-fallback" => "Sent as a request:",
        _ => unreachable!(),
    }
}

#[test]
fn every_feedback_state_has_approved_exchange_colours_at_mock_widths() {
    // The frozen phase is the mock's representative ⠹ frame; elapsed stays 0:42.
    const NOW: u64 = 1_791_600_000_160;
    crate::status::with_now_ms(NOW, || {
        let mut frames = Vec::new();
        for (look_name, look) in crate::board::home::tests::oracle::looks() {
            for scene in SCENES {
                for width in [103, 148] {
                    let mut app = fixture(scene, look, NOW);
                    if scene == "chat-fallback" {
                        let projection = app.composer_projection().unwrap();
                        assert_eq!(
                            app.chat_fallback(
                                projection.key.as_ref().unwrap(),
                                projection.recipient
                            )
                            .as_deref(),
                            Some(
                                "Sent as a request: night-2 can't take a direct message right now."
                            )
                        );
                    }
                    let _no_acquisition = crate::runner::forbid_commands();
                    let mut terminal = Terminal::new(TestBackend::new(width, 51)).unwrap();
                    terminal
                        .draw(|frame| {
                            crate::board::view::prepare(&mut app, frame.area());
                            crate::board::view::render(frame, &app);
                        })
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let lines: Vec<String> = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                        .collect();
                    let text = lines.join("\n");
                    let recipient = if scene == "sent-unknown" {
                        "mkt-gemini"
                    } else {
                        "night-2"
                    };
                    let incoming = format!("{recipient} → you");
                    let incoming_headers: Vec<_> = lines
                        .iter()
                        .enumerate()
                        .filter(|(_, line)| line.contains(&incoming))
                        .collect();
                    assert_eq!(
                        incoming_headers.len(),
                        usize::from(scene == "sent-replied"),
                        "only the settled reply owns an incoming heading: {text}"
                    );
                    if let Some((y, header)) = incoming_headers.first() {
                        let x = header[..header.find(&incoming).unwrap()].width();
                        for cell in &buffer.content[y * usize::from(width) + x
                            ..y * usize::from(width) + x + recipient.width()]
                        {
                            assert_eq!(
                                cell.fg,
                                look.role(Role::Text)
                                    .fg
                                    .unwrap_or(ratatui::style::Color::Reset)
                            );
                            assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD));
                        }
                    }
                    let needle = expected(scene);
                    let (y, start) = lines
                        .iter()
                        .enumerate()
                        .find_map(|(y, line)| line.find(needle).map(|start| (y, start)))
                        .unwrap_or_else(|| {
                            panic!("{look_name}/{scene}/{width}: missing {needle}\n{text}")
                        });
                    if scene.starts_with("chat-") {
                        assert!(
                            !text.contains("is working")
                                && !text.contains("finished without replying"),
                            "Core capture, not provider feedback: {text}"
                        );
                        if scene == "chat-answered" || scene == "chat-truncated" {
                            assert!(text.contains(REPLY));
                        }
                    } else if scene != "sent-replied" {
                        let x = lines[y][..start].width();
                        let working_at = needle
                            .find("is working")
                            .map(|start| needle[..start].width());
                        for (offset, cell) in buffer.content[y * usize::from(width) + x
                            ..y * usize::from(width) + x + needle.width()]
                            .iter()
                            .enumerate()
                        {
                            let role = if scene == "sent-working"
                                && working_at.is_some_and(|start| {
                                    (start..start + "is working".len()).contains(&offset)
                                })
                                || matches!(scene, "sent-finished" | "sent-unknown") && offset == 0
                            {
                                Role::Working
                            } else {
                                Role::Dim
                            };
                            assert_eq!(
                                cell.fg,
                                look.role(role).fg.unwrap_or(ratatui::style::Color::Reset),
                                "{look_name}/{scene}/{width}: feedback cell {offset}"
                            );
                        }
                        if scene == "sent-working" {
                            let spinner = &buffer.content[y * usize::from(width) + x - 2];
                            assert_eq!(
                                spinner.fg,
                                look.role(Role::Working)
                                    .fg
                                    .unwrap_or(ratatui::style::Color::Reset)
                            );
                        }
                    } else {
                        assert!(
                            !text.contains("is working")
                                && !text.contains("activity is not reported")
                                && !text.contains("finished without replying"),
                            "{text}"
                        );
                    }
                    let (header_y, header) = lines
                        .iter()
                        .enumerate()
                        .find(|(_, line)| line.contains("you → "))
                        .expect("outgoing sender header");
                    let sender_x = header[..header.find("you → ").unwrap()].width();
                    for cell in &buffer.content[header_y * usize::from(width) + sender_x
                        ..header_y * usize::from(width) + sender_x + 3]
                    {
                        assert_eq!(
                            cell.fg,
                            look.role(Role::Accent)
                                .fg
                                .unwrap_or(ratatui::style::Color::Reset)
                        );
                        assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD));
                    }
                    let recipient_x = header[..header.find(recipient).unwrap()].width();
                    for cell in &buffer.content[header_y * usize::from(width) + recipient_x
                        ..header_y * usize::from(width) + recipient_x + recipient.width()]
                    {
                        assert_eq!(
                            cell.fg,
                            look.role(Role::Text)
                                .fg
                                .unwrap_or(ratatui::style::Color::Reset)
                        );
                    }
                    for line in &lines {
                        assert_eq!(
                            crate::board::glyph_guard::glyph_error(line),
                            None,
                            "{look_name}/{scene}/{width}: {line}"
                        );
                    }
                    frames.push(json!({"scene":scene,"look":look_name,"width":width,"height":51,
                        "lines":lines,"cells":buffer.content.iter().map(|cell|json!([cell.symbol(),format!("{:?}",cell.fg),format!("{:?}",cell.bg),format!("{:?}",cell.modifier)])).collect::<Vec<_>>(),
                        "input":format!("{:?}",app.input_band.get()),"focus":format!("{:?}",app.focused_pane())}));
                }
            }
        }
        if let Some(path) = std::env::var_os("TMT_OUTGOING_FEEDBACK_CAPTURE") {
            std::fs::write(path,serde_json::to_vec(&json!({"source":"Approved Colab739017fe exchange colours; Colab6e70ab55 activity states; accepted-send composer explicitly closed to show idle input; ordinary request feedback and Core capture states; literal fixture answer", "frames":frames})).unwrap()).unwrap();
        }
    });
}

#[test]
fn core_text_changes_reach_live_and_restored_chat_without_local_activity_inference() {
    const NOW: u64 = 1_791_600_000_160;
    crate::status::with_now_ms(NOW, || {
        for scene in [
            "chat-bound",
            "chat-answered",
            "chat-no-answer",
            "chat-truncated",
        ] {
            let mut app = fixture(scene, Look::default(), NOW);
            let _ = board_frame(&app, 160, 51);
            let text = "Core changed its wording: waiting for permission.";
            let owner = app
                .conversation_finals()
                .expect("visible conversation lease");
            app.apply_sent(
            crate::board::home_leads::SentKey {
                sender: owner.sender,
                agent: owner.agent,
            },
            Ok(vec![json!({
                "requestId":REQUEST,"preparedAtMs":NOW-42_000,"text":QUESTION,"truncated":false,
                "mode":"passthrough","passthrough":{"state":match scene {"chat-bound"=>"bound","chat-no-answer"=>"no_answer",_=>"answered"},
                    "answer":{"text":REPLY,"truncated":scene=="chat-truncated","provider":"codex","endedMs":NOW-1000}},
                "messageState":{"state":"working","reason":{"text":text}}
            })]),
        );
            let _ = board_frame(&app, 160, 51);
            {
                let mut state = app.conversation.borrow_mut();
                let session = state.sessions.values_mut().next().unwrap();
                session.observe_progress("night-2", None, NOW, true);
                assert_eq!(session.outgoing[0].status(), format!("⠹ {text}"));
                // Core's words stay the status; only the leading working mark moves.
                assert!(matches!(
                    session.outgoing[0]
                        .feedback_clock
                        .as_ref()
                        .map(|c| c.progress),
                    Some(crate::board::conversation::outgoing::progress::Progress::Stated { .. })
                ));
                assert!(session.outgoing[0].capture.is_some());
            }
            let live = board_frame(&app, 160, 51);
            assert!(live.contains(text), "{live}");
            assert!(!live.contains("working…"), "{live}");
            {
                let mut state = app.conversation.borrow_mut();
                let session = state.sessions.values_mut().next().unwrap();
                session.outgoing.clear();
                session.refresh_outgoing();
            }
            let restored = board_frame(&app, 160, 51);
            assert!(restored.contains(text), "{restored}");
            assert!(!restored.contains("working…"), "{restored}");
            if scene != "chat-bound" {
                assert!(live.contains(expected(scene)), "{live}");
                assert!(restored.contains(expected(scene)), "{restored}");
            }
        }
    });
}

#[test]
fn core_text_overrides_every_accepted_disposition_but_missing_state_keeps_the_local_line() {
    for disposition in [
        Disposition::Sent,
        Disposition::Queued,
        Disposition::Offline,
        Disposition::Held,
        Disposition::Uncertain,
    ] {
        let app = fixture("chat-bound", Look::default(), 1_791_600_000_160);
        let mut state = app.conversation.borrow_mut();
        let entry = &mut state.sessions.values_mut().next().unwrap().outgoing[0];
        let Some(Ok(accepted)) = &mut entry.outcome else {
            panic!("accepted fixture")
        };
        accepted.disposition = disposition;
        accepted.passthrough = false;
        accepted.message_state_label = Some("Core owns these exact words.".into());
        entry.sending_held = true;
        assert_eq!(entry.status(), "Core owns these exact words.");
        entry.sending_held = false;
        let Some(Ok(accepted)) = &mut entry.outcome else {
            unreachable!()
        };
        accepted.message_state_label = None;
        entry.feedback = "legacy delivered line".into();
        assert!(!entry.status().is_empty());
    }
}

/// The public packet has an observation clock before Core's settlement clock.
fn window_capture_fixture(restored: bool, look: Look, now: u64) -> App {
    let mut app = fixture("chat-answered", look, now);
    board_frame(&app, 160, 51);
    let owner = app.conversation_finals().unwrap();
    app.apply_sent(
        crate::board::home_leads::SentKey {
            sender: owner.sender,
            agent: owner.agent,
        },
        Ok(vec![json!({
            "requestId":REQUEST,"preparedAtMs":now-42_000,"text":QUESTION,"truncated":false,
            "mode":"passthrough","messageState":{"state":"delivered","reason":{"text":"delivered"}},
            "passthrough":{"state":"answered","boundAtMs":now-40_000,"settledAtMs":now-1_000,
                "answer":{"text":REPLY,"truncated":false,"provider":"codex","endedMs":now-1_000}}
        })]),
    );
    board_frame(&app, 160, 51);
    if restored {
        let mut state = app.conversation.borrow_mut();
        let session = state.sessions.values_mut().next().unwrap();
        session.outgoing.clear();
        session.refresh_outgoing();
    }
    app
}

#[test]
fn one_provider_final_with_different_observation_and_settlement_clocks_paints_once() {
    const NOW: u64 = 1_791_600_000_160;
    crate::status::with_now_ms(NOW, || {
        for restored in [false, true] {
            let app = window_capture_fixture(restored, Look::default(), NOW);
            observe_pane(&app, vec![window_item(NOW)]);
            let frame = board_frame(&app, 160, 51);
            assert_eq!(
                frame.matches(REPLY).count(),
                1,
                "restored={restored}: {frame}"
            );
            assert!(frame.contains("turn output"), "{frame}");
            assert!(!frame.contains("pane output"), "{frame}");
            let state = app.conversation.borrow();
            let session = state.sessions.get(state.active.as_ref().unwrap()).unwrap();
            let rendered = session.rendered.as_ref().unwrap();
            let row = rendered
                .mapped
                .lines
                .iter()
                .position(|line| line.to_string().contains(REPLY))
                .unwrap();
            assert!(
                session
                    .response_key_at(rendered.mapped.sources[row])
                    .is_none(),
                "a capture is not a reply source"
            );
            assert_eq!(rendered.copy.lines.len(), rendered.mapped.lines.len());
            assert!(rendered.copy.rows[row].contains(REPLY));
            let shown = vec![rendered.copy.rows[row].clone()];
            let copied = super::super::super::copy::passage(
                &rendered.copy,
                &super::super::super::copy::Selected {
                    first: row,
                    last: row,
                    shown: &shown,
                    start: super::super::super::copy::End {
                        row: &shown[0],
                        at: 0,
                    },
                    end: super::super::super::copy::End {
                        row: &shown[0],
                        at: shown[0].len(),
                    },
                },
            );
            assert!(copied.contains(REPLY));
            assert!(
                rendered
                    .anchor_text
                    .line(rendered.anchor_sources[row])
                    .unwrap()
                    .contains(REPLY)
            );
        }
    });
}

fn window_item(now: u64) -> crate::board::recent_responses::Item {
    crate::board::recent_responses::Item {
        text: REPLY.into(),
        provider: "codex".into(),
        ended_ms: now - 1_007,
        truncated: false,
    }
}

fn observe_pane(app: &App, items: Vec<crate::board::recent_responses::Item>) {
    app.conversation.borrow_mut().refresh_pane_output();
    let key = app.pane_output_read().unwrap();
    app.apply_pane_output(
        key,
        crate::board::recent_responses::Observation::Observed(items),
    );
}

#[test]
fn another_turn_with_identical_text_is_kept_and_ambiguous_observations_are_not_hidden() {
    const NOW: u64 = 1_791_600_000_160;
    crate::status::with_now_ms(NOW, || {
        for restored in [false, true] {
            let app = window_capture_fixture(restored, Look::default(), NOW);
            let matched = window_item(NOW);
            let mut earlier = matched.clone();
            earlier.ended_ms = NOW - 50_000; // A different turn, outside the capture window.
            observe_pane(&app, vec![earlier, matched.clone()]);
            let frame = board_frame(&app, 160, 51);
            assert_eq!(frame.matches(REPLY).count(), 2, "{frame}");
            assert_eq!(frame.matches("pane output").count(), 1, "{frame}");
            observe_pane(&app, vec![matched.clone(), matched]);
            let frame = board_frame(&app, 160, 51);
            assert_eq!(
                frame.matches(REPLY).count(),
                3,
                "two indistinguishable pane turns remain: {frame}"
            );
            assert_eq!(frame.matches("pane output").count(), 2, "{frame}");
        }
    });
}

#[test]
fn capture_pairing_requires_exact_metadata_and_valid_bounds_without_a_tolerance() {
    const NOW: u64 = 1_791_600_000_160;
    crate::status::with_now_ms(NOW, || {
        let app = window_capture_fixture(true, Look::default(), NOW);
        let mut state = app.conversation.borrow_mut();
        let session = state.sessions.values_mut().next().unwrap();
        let matching = window_item(NOW);
        assert_eq!(
            session.captured_outputs(std::slice::from_ref(&matching)),
            [0]
        );
        let mut other = matching.clone();
        other.provider = "claude".into();
        assert!(session.captured_outputs(&[other]).is_empty());
        let mut other = matching.clone();
        other.text.push(' ');
        assert!(session.captured_outputs(&[other]).is_empty());
        let mut other = matching.clone();
        other.truncated = true;
        assert!(session.captured_outputs(&[other]).is_empty());
        for at in [NOW - 40_000, NOW - 1_000] {
            let mut edge = matching.clone();
            edge.ended_ms = at;
            assert_eq!(session.captured_outputs(&[edge]), [0]);
        }
        for at in [NOW - 40_001, NOW - 999] {
            let mut outside = matching.clone();
            outside.ended_ms = at;
            assert!(session.captured_outputs(&[outside]).is_empty());
        }
        let original = session.sent[0].capture.clone().unwrap();
        for (bound, settled) in [
            (None, Some(NOW - 1_000)),
            (Some(NOW - 40_000), None),
            (Some(NOW), Some(NOW - 1_000)),
        ] {
            let mut changed = original.clone();
            let crate::board::row_detail::Capture::Answered {
                bound_ms,
                settled_ms,
                ..
            } = &mut changed
            else {
                unreachable!()
            };
            *bound_ms = bound;
            *settled_ms = settled;
            session.sent[0].capture = Some(changed);
            assert!(
                session
                    .captured_outputs(std::slice::from_ref(&matching))
                    .is_empty()
            );
            let mut exact = matching.clone();
            exact.ended_ms = NOW - 1_000;
            assert_eq!(
                session.captured_outputs(&[exact]),
                [0],
                "legacy exact observation still works"
            );
        }
        session.sent[0].capture = Some(original);
        let mut second = session.sent[0].clone();
        second.request = "another-request".into();
        session.sent.push(second);
        assert!(
            session.captured_outputs(&[matching]).is_empty(),
            "two owned captures cannot claim one observation"
        );
    });
}

#[test]
fn an_unavailable_pane_refresh_keeps_the_owned_turn_output_and_reconciles_again() {
    const NOW: u64 = 1_791_600_000_160;
    crate::status::with_now_ms(NOW, || {
        let app = window_capture_fixture(false, Look::default(), NOW);
        observe_pane(&app, vec![window_item(NOW)]);
        assert_eq!(board_frame(&app, 160, 51).matches(REPLY).count(), 1);
        app.conversation.borrow_mut().refresh_pane_output();
        let key = app.pane_output_read().unwrap();
        app.apply_pane_output(
            key,
            crate::board::recent_responses::Observation::Unavailable,
        );
        let frame = board_frame(&app, 160, 51);
        assert_eq!(frame.matches(REPLY).count(), 1, "{frame}");
        assert!(frame.contains("Recent pane output unavailable"), "{frame}");
        observe_pane(&app, vec![window_item(NOW)]);
        let frame = board_frame(&app, 160, 51);
        assert_eq!(frame.matches(REPLY).count(), 1, "{frame}");
        assert!(!frame.contains("Recent pane output unavailable"), "{frame}");
    });
}

const TURN_REQUESTS: [&str; 3] = [
    "req_61111111-1111-4111-8111-111111111111",
    "req_62222222-2222-4222-8222-222222222222",
    "req_63333333-3333-4333-8333-333333333333",
];
const TURN_TEXTS: [&str; 3] = [
    "first question about cpu",
    "second question about the night",
    "third question about the gui seat",
];
const TURN_ANSWER: &str = "One answer to all three questions.";

/// Oracle scenes: several sends answered by one turn, painted as one message block (live and
/// restored), a block still working, a long body between two short ones, and a failed send in
/// place.
pub(in crate::board) const ONE_TURN_SCENES: [&str; 5] = [
    "chat-one-turn",
    "chat-one-turn-restored",
    "chat-block-working",
    "chat-block-paragraphs",
    "chat-block-failed",
];

pub(in crate::board) fn one_turn_scene(scene: &str, look: Look, now: u64) -> App {
    let mut app = match scene {
        "chat-block-working" => sends_scene(false, &working_specs(now)),
        "chat-block-paragraphs" => sends_scene(false, &paragraph_specs(now)),
        "chat-block-failed" => sends_scene(false, &failed_specs(now)),
        _ => one_turn_for_three_sends(scene.ends_with("-restored"), now),
    };
    app.view.as_mut().unwrap().look = look;
    board_frame(&app, 160, 60);
    app
}

/// Three direct sends, the last two typed while the agent worked on the first; one turn answered
/// them all, so Core bound it to the first only.
fn one_turn_for_three_sends(restored: bool, now: u64) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    let view = app.view.as_mut().unwrap();
    view.me = Some("you".into());
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    // Prepared 120 s, 90 s and 70 s ago; the turn settled 1 s ago.
    let sent = [now - 120_000, now - 90_000, now - 70_000];
    for index in 0..3 {
        let (ticket, _) =
            crate::status::with_now_ms(sent[index], || begin(&mut app, TURN_TEXTS[index]));
        finish(
            &mut app,
            Completed {
                ticket: Some(ticket),
                outcome: Ok(Accepted {
                    request: TURN_REQUESTS[index].into(),
                    disposition: Disposition::Sent,
                    passthrough: true,
                    message_state_label: Some("✓ delivered".into()),
                    can_send_now: false,
                    working: false,
                    fallback: None,
                    warning: None,
                }),
                what: "Message".into(),
                to: "worker".into(),
            },
        );
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.notice = None;
    }
    board_frame(&app, 160, 60);
    let owner = app.conversation_finals().unwrap();
    let send = |index: usize, passthrough: serde_json::Value| {
        json!({
            "requestId": TURN_REQUESTS[index], "preparedAtMs": sent[index],
            "text": TURN_TEXTS[index], "truncated": false, "mode": "passthrough",
            "messageState": {"state": "delivered", "reason": {"text": "delivered"}},
            "passthrough": passthrough
        })
    };
    app.apply_sent(
        crate::board::home_leads::SentKey {
            sender: owner.sender,
            agent: owner.agent,
        },
        Ok(vec![
            send(
                0,
                json!({"state":"answered","boundAtMs":now - 119_000,"settledAtMs":now - 1_000,
                    "answer":{"text":TURN_ANSWER,"truncated":false,"provider":"claude","endedMs":now - 1_000}}),
            ),
            send(1, json!({"state":"no_answer"})),
            send(2, json!({"state":"no_answer"})),
        ]),
    );
    if restored {
        let mut state = app.conversation.borrow_mut();
        let session = state.sessions.values_mut().next().unwrap();
        session.outgoing.clear();
        session.refresh_outgoing();
    }
    app
}

/// Regression guard for several sends typed during one agent turn (#4631): the turn's one answer
/// is bound to the first send, and must be painted under the last send it covers.
fn assert_one_turn_answer_is_under_the_last_send(restored: bool) {
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        let app = one_turn_for_three_sends(restored, NOW);
        // The first frame prepares the new text; the second paints it.
        board_frame(&app, 160, 60);
        let frame = board_frame(&app, 160, 60);
        let at = |needle: &str| {
            frame
                .find(needle)
                .unwrap_or_else(|| panic!("restored={restored}: {needle:?} missing\n{frame}"))
        };
        assert_eq!(
            frame.matches(TURN_ANSWER).count(),
            1,
            "restored={restored}: the answer paints once\n{frame}"
        );
        assert!(
            at(TURN_TEXTS[0]) < at(TURN_TEXTS[1])
                && at(TURN_TEXTS[1]) < at(TURN_TEXTS[2])
                && at(TURN_TEXTS[2]) < at(TURN_ANSWER),
            "restored={restored}: the answer follows the last send\n{frame}"
        );
        assert_eq!(
            frame.matches("you → worker").count(),
            1,
            "restored={restored}: one header for the block\n{frame}"
        );
        assert_eq!(
            frame.matches("✓ delivered").count(),
            1,
            "restored={restored}: one status line for the block\n{frame}"
        );
        assert!(
            at("✓ delivered") > at(TURN_TEXTS[2]) && at("✓ delivered") < at(TURN_ANSWER),
            "restored={restored}: the status sits between the last body and the answer\n{frame}"
        );
        assert!(
            !frame.contains("answered below"),
            "restored={restored}: a block needs no pointer to its answer\n{frame}"
        );
    });
}

#[test]
fn several_sends_typed_during_one_turn_paint_its_answer_under_the_last_in_live_chat() {
    assert_one_turn_answer_is_under_the_last_send(false);
}

#[test]
fn several_sends_typed_during_one_turn_paint_its_answer_under_the_last_in_restored_chat() {
    assert_one_turn_answer_is_under_the_last_send(true);
}

/// One direct send for `sends_scene`.
struct Spec {
    text: &'static str,
    at_ms: u64,
    failed: bool,
    /// Core's `passthrough` object and `messageState` text for the send.
    passthrough: serde_json::Value,
    state: (&'static str, &'static str),
}

fn spec(text: &'static str, at_ms: u64, passthrough: serde_json::Value) -> Spec {
    Spec {
        text,
        at_ms,
        failed: false,
        passthrough,
        state: ("delivered", "delivered"),
    }
}

fn sends_scene(restored: bool, specs: &[Spec]) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    let view = app.view.as_mut().unwrap();
    view.me = Some("you".into());
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    let request = |index: usize| format!("req_7{index:07}-1111-4111-8111-111111111111");
    for (index, spec) in specs.iter().enumerate() {
        let (ticket, _) = crate::status::with_now_ms(spec.at_ms, || begin(&mut app, spec.text));
        let outcome = if spec.failed {
            Err(crate::core::SquadError::new("SEND_FAILED", "not delivered"))
        } else {
            Ok(Accepted {
                request: request(index),
                disposition: Disposition::Sent,
                passthrough: true,
                message_state_label: Some("✓ delivered".into()),
                can_send_now: false,
                working: false,
                fallback: None,
                warning: None,
            })
        };
        finish(
            &mut app,
            Completed {
                ticket: Some(ticket),
                outcome,
                what: "Message".into(),
                to: "worker".into(),
            },
        );
        if spec.failed {
            // The kept draft is cleared, as a person would, so the next message starts empty.
            app.input.as_mut().unwrap().text.clear();
        }
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.notice = None;
    }
    board_frame(&app, 160, 60);
    let owner = app.conversation_finals().unwrap();
    let sent: Vec<_> = specs
        .iter()
        .enumerate()
        .filter(|(_, spec)| !spec.failed)
        .map(|(index, spec)| {
            json!({
                "requestId": request(index), "preparedAtMs": spec.at_ms,
                "text": spec.text, "truncated": false, "mode": "passthrough",
                "messageState": {"state": spec.state.0, "reason": {"text": spec.state.1}},
                "passthrough": spec.passthrough
            })
        })
        .collect();
    app.apply_sent(
        crate::board::home_leads::SentKey {
            sender: owner.sender,
            agent: owner.agent,
        },
        Ok(sent),
    );
    if restored {
        let mut state = app.conversation.borrow_mut();
        let session = state.sessions.values_mut().next().unwrap();
        session.outgoing.clear();
        session.refresh_outgoing();
    }
    app
}

fn rows(app: &App, width: u16, height: u16) -> Vec<String> {
    // The first frame prepares new text; the second paints it.
    board_frame(app, width, height);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}

fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} missing\n{}", rows.join("\n")))
}

fn count_rows(rows: &[String], needle: &str) -> usize {
    rows.iter().filter(|row| row.contains(needle)).count()
}

const WORKING: &str = r#"{"state":"bound"}"#;

fn working_specs(now: u64) -> Vec<Spec> {
    let mut first = spec(
        "first message",
        now - 30_000,
        serde_json::from_str(WORKING).unwrap(),
    );
    first.state = ("working", "working…");
    vec![
        first,
        spec("second message", now - 20_000, json!({"state":"pending"})),
        spec("third message", now - 10_000, json!({"state":"pending"})),
    ]
}

fn paragraph_specs(now: u64) -> Vec<Spec> {
    vec![
        spec(
            "short opener",
            now - 120_000,
            json!({"state":"answered","boundAtMs":now - 119_000,"settledAtMs":now - 1_000,
                "answer":{"text":TURN_ANSWER,"truncated":false,"provider":"claude","endedMs":now - 1_000}}),
        ),
        spec(
            "paragraph one of the long message\n\nparagraph two of the long message\n\nparagraph three of the long message",
            now - 90_000,
            json!({"state":"no_answer"}),
        ),
        spec("short closer", now - 70_000, json!({"state":"no_answer"})),
    ]
}

fn failed_specs(now: u64) -> Vec<Spec> {
    let mut failed = spec("broken message", now - 20_000, json!(null));
    failed.failed = true;
    vec![
        spec(
            "first message",
            now - 30_000,
            serde_json::from_str(WORKING).unwrap(),
        ),
        failed,
        spec("third message", now - 10_000, json!({"state":"pending"})),
    ]
}

#[test]
fn sends_typed_while_the_turn_works_read_as_one_block_with_one_working_status() {
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        for restored in [false, true] {
            let mut first = spec(
                "first message",
                NOW - 30_000,
                serde_json::from_str(WORKING).unwrap(),
            );
            first.state = ("working", "working…");
            let specs = [
                first,
                spec("second message", NOW - 20_000, json!({"state":"pending"})),
                spec("third message", NOW - 10_000, json!({"state":"pending"})),
            ];
            let app = sends_scene(restored, &specs);
            let rows = rows(&app, 160, 60);
            let shown = rows.join("\n");
            assert_eq!(
                count_rows(&rows, "you → worker"),
                1,
                "restored={restored}: {shown}"
            );
            assert_eq!(
                count_rows(&rows, "⠹ working…"),
                1,
                "restored={restored}: {shown}"
            );
            assert_eq!(
                count_rows(&rows, "✓ delivered"),
                0,
                "restored={restored}: {shown}"
            );
            let (a, b, c) = (
                row_of(&rows, "first message"),
                row_of(&rows, "second message"),
                row_of(&rows, "third message"),
            );
            assert_eq!(
                (b - a, c - b),
                (1, 1),
                "restored={restored}: each body starts on the next line, no separator\n{shown}"
            );
            assert!(
                row_of(&rows, "⠹ working…") > c,
                "restored={restored}: the status closes the block\n{shown}"
            );
            assert_eq!(count_rows(&rows, "turn output"), 0, "no answer yet");
        }
    });
}

#[test]
fn a_send_after_the_answer_settled_starts_a_new_block() {
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        for restored in [false, true] {
            let specs = [
                spec(
                    "first message",
                    NOW - 120_000,
                    json!({"state":"answered","boundAtMs":NOW - 119_000,"settledAtMs":NOW - 60_000,
                        "answer":{"text":TURN_ANSWER,"truncated":false,"provider":"claude","endedMs":NOW - 60_000}}),
                ),
                spec("second message", NOW - 90_000, json!({"state":"no_answer"})),
                spec("after the answer", NOW - 30_000, json!({"state":"pending"})),
            ];
            let app = sends_scene(restored, &specs);
            let rows = rows(&app, 160, 60);
            let shown = rows.join("\n");
            assert_eq!(
                count_rows(&rows, "you → worker"),
                2,
                "restored={restored}: {shown}"
            );
            let (a, b, answer, later) = (
                row_of(&rows, "first message"),
                row_of(&rows, "second message"),
                row_of(&rows, TURN_ANSWER),
                row_of(&rows, "after the answer"),
            );
            assert!(
                a < b && b < answer && answer < later,
                "restored={restored}: {shown}"
            );
            assert_eq!(count_rows(&rows, TURN_ANSWER), 1);
        }
    });
}

#[test]
fn a_failed_send_stays_visible_in_place_with_its_own_error_and_ends_the_block() {
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        let mut failed = spec("broken message", NOW - 20_000, json!(null));
        failed.failed = true;
        let specs = [
            spec(
                "first message",
                NOW - 30_000,
                serde_json::from_str(WORKING).unwrap(),
            ),
            failed,
            spec("third message", NOW - 10_000, json!({"state":"pending"})),
        ];
        let app = sends_scene(false, &specs);
        let rows = rows(&app, 160, 60);
        let shown = rows.join("\n");
        assert_eq!(count_rows(&rows, "you → worker"), 3, "{shown}");
        let (broken, error, third) = (
            row_of(&rows, "broken message"),
            row_of(&rows, "✗ not delivered"),
            row_of(&rows, "third message"),
        );
        assert!(
            broken < error && error < third,
            "the error line sits under the failed message: {shown}"
        );
    });
}

#[test]
fn a_multi_paragraph_body_is_set_off_by_a_blank_line_from_its_short_neighbours() {
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        for restored in [false, true] {
            let specs = [
                spec(
                    "short opener",
                    NOW - 120_000,
                    json!({"state":"answered","boundAtMs":NOW - 119_000,"settledAtMs":NOW - 1_000,
                        "answer":{"text":TURN_ANSWER,"truncated":false,"provider":"claude","endedMs":NOW - 1_000}}),
                ),
                spec(
                    "paragraph one of the long message\n\nparagraph two of the long message\n\nparagraph three of the long message",
                    NOW - 90_000,
                    json!({"state":"no_answer"}),
                ),
                spec("short closer", NOW - 70_000, json!({"state":"no_answer"})),
            ];
            let app = sends_scene(restored, &specs);
            let rows = rows(&app, 160, 60);
            let shown = rows.join("\n");
            let (opener, one, two, three, closer) = (
                row_of(&rows, "short opener"),
                row_of(&rows, "paragraph one"),
                row_of(&rows, "paragraph two"),
                row_of(&rows, "paragraph three"),
                row_of(&rows, "short closer"),
            );
            assert_eq!(
                one - opener,
                2,
                "restored={restored}: a blank line before the long body\n{shown}"
            );
            assert_eq!(
                two - one,
                2,
                "restored={restored}: its own paragraph gap stays\n{shown}"
            );
            assert_eq!(three - two, 2, "restored={restored}: {shown}");
            assert_eq!(
                closer - three,
                2,
                "restored={restored}: and after it\n{shown}"
            );
            assert!(row_of(&rows, TURN_ANSWER) > closer, "{shown}");
        }
    });
}

#[test]
fn every_message_of_a_block_keeps_its_own_source_lines_for_copy_and_quote() {
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        for restored in [false, true] {
            let specs = [
                spec(
                    "short opener",
                    NOW - 120_000,
                    json!({"state":"answered","boundAtMs":NOW - 119_000,"settledAtMs":NOW - 1_000,
                        "answer":{"text":TURN_ANSWER,"truncated":false,"provider":"claude","endedMs":NOW - 1_000}}),
                ),
                spec(
                    "paragraph one of the long message\n\nparagraph two of the long message",
                    NOW - 90_000,
                    json!({"state":"no_answer"}),
                ),
                spec("short closer", NOW - 70_000, json!({"state":"no_answer"})),
            ];
            let app = sends_scene(restored, &specs);
            rows(&app, 160, 60);
            let state = app.conversation.borrow();
            let session = state.sessions.get(state.active.as_ref().unwrap()).unwrap();
            let rendered = session.rendered.as_ref().unwrap();
            for needle in ["short opener", "paragraph two", "short closer"] {
                let row = rendered
                    .mapped
                    .lines
                    .iter()
                    .position(|line| line.to_string().contains(needle))
                    .unwrap_or_else(|| panic!("{needle:?} not painted"));
                let at = rendered.copy.lines[row].unwrap_or_else(|| {
                    panic!("restored={restored}: {needle:?} has no copy source")
                });
                assert!(
                    rendered.copy.messages[at.message]
                        .lines()
                        .nth(at.line)
                        .is_some_and(|line| line.contains(needle)),
                    "restored={restored}: {needle:?} copies from its own message line"
                );
            }
        }
    });
}
