//! Six sent-message states through the real composer and held observation ports.
//! The frozen oracle and task-owned mock-width captures use the same fixtures.
use super::*;
use crate::board::{activity::Phase, app::workspace::tests::MEMBER};
use crate::look::Look;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) const SCENES: [&str; 6] = [
    "sent-sending",
    "sent-working",
    "sent-earlier",
    "sent-replied",
    "sent-finished",
    "sent-unknown",
];
const QUESTION: &str = "Can you rerun the picker tests on main?";
const REPLY: &str = "Yes. All 14 picker tests pass on main 6899a9f2.";
const REQUEST: &str = "req_55555555-5555-4555-8555-555555555555";

pub(in crate::board) fn fixture(scene: &str, look: Look, now_ms: u64) -> App {
    assert!(SCENES.contains(&scene));
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
        "sent-sending" => now_ms,
        "sent-earlier" => now_ms - 42_000,
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

fn expected(scene: &str) -> &'static str {
    match scene {
        "sent-sending" => "sending…",
        "sent-working" => "night-2 is working · 0:42",
        "sent-earlier" => "◌ night-2 is finishing an earlier turn · 3m",
        "sent-replied" => REPLY,
        "sent-finished" => "✓ night-2 finished without replying · 2m ago",
        "sent-unknown" => "✓ sent · mkt-gemini's activity is not reported",
        _ => unreachable!(),
    }
}

#[test]
fn every_feedback_state_has_neutral_mock_width_frames() {
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
                    assert!(
                        !text.contains(&format!("{recipient} → you")),
                        "first outgoing turn has no empty incoming heading: {text}"
                    );
                    let needle = expected(scene);
                    let (y, start) = lines
                        .iter()
                        .enumerate()
                        .find_map(|(y, line)| line.find(needle).map(|start| (y, start)))
                        .unwrap_or_else(|| {
                            panic!("{look_name}/{scene}/{width}: missing {needle}\n{text}")
                        });
                    if scene != "sent-replied" {
                        let x = lines[y][..start].width();
                        let dim = look
                            .role(Role::Dim)
                            .fg
                            .unwrap_or(ratatui::style::Color::Reset);
                        for cell in &buffer.content[y * usize::from(width) + x
                            ..y * usize::from(width) + x + needle.width()]
                        {
                            assert_eq!(
                                cell.fg, dim,
                                "{look_name}/{scene}/{width}: feedback is neutral"
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
            std::fs::write(path,serde_json::to_vec(&json!({"source":"Approved Colab6e70ab55; accepted-send composer explicitly closed to show idle input; real context prefix retained; literal fixture reply", "frames":frames})).unwrap()).unwrap();
        }
    });
}
