//! Six sent-message states through the real composer and held observation ports.
//! The frozen oracle and task-owned mock-width captures use the same fixtures.
use super::*;
use crate::board::{activity::Phase, app::workspace::tests::MEMBER};
use crate::look::Look;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) const SCENES: [&str; 12] = [
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
];
const QUESTION: &str = "Can you rerun the picker tests on main?";
const REPLY: &str = "Yes. All 14 picker tests pass on main 6899a9f2.";
const REQUEST: &str = "req_55555555-5555-4555-8555-555555555555";

pub(in crate::board) fn fixture(scene: &str, look: Look, now_ms: u64) -> App {
    assert!(SCENES.contains(&scene));
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
            "chat-pending" => "pending",
            "chat-bound" => "bound",
            "chat-no-answer" => "no_answer",
            _ => "answered",
        };
        app.apply_sent(crate::board::home_leads::SentKey { sender, agent: MEMBER.into() }, Ok(vec![json!({
            "requestId":REQUEST,"preparedAtMs":at,"text":QUESTION,"truncated":false,"mode":"passthrough",
            "passthrough":{"state":state,"boundAtMs":at+1,"answer":{"text":REPLY,"truncated":scene=="chat-truncated","provider":"codex","endedMs":now_ms-1000}}
        })]));
    }
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.notice = None;
    app.focus = 1;
    app
}

fn expected(scene: &str) -> &'static str {
    match scene {
        "sent-sending" => "sending…",
        "sent-working" => "night-2 is working · 0:42",
        "sent-earlier" => "◌ night-2 is finishing an earlier turn · 3m",
        "sent-replied" => REPLY,
        "sent-finished" => "✓ night-2 finished without replying · 2m ago",
        "sent-unknown" => "✓ sent · mkt-gemini's activity is not reported",
        "chat-pending" | "chat-bound" => "working…",
        "chat-answered" => "turn output",
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
