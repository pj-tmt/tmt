//! Explicit test-owned look evidence; never modifies a frozen fixture.
use super::*;

#[test]
#[ignore = "exports Markdown look scenes only to an explicitly supplied task-owned path"]
fn capture_conversation_markdown_packet() {
    let output = std::env::var_os("TMT_MARKDOWN_PACKET").expect("task-owned packet path");
    let _no_commands = crate::runner::forbid_commands();
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
        for width in [103, 148] {
            for scene in [
                "inline",
                "code",
                "quote",
                "table",
                "table-fallback",
                "folded",
                "expanded",
            ] {
                let mut app = if matches!(scene, "folded" | "expanded") {
                    long_outgoing_app()
                } else {
                    fixture()
                };
                app.view.as_mut().unwrap().look = Look {
                    theme: tmt_cli_style::Theme::new(base),
                    depth,
                };
                if !matches!(scene, "folded" | "expanded") {
                    let body = match scene {
                        "inline" => "# Delivery plan\n\n- **Ready** for review\n- Read `config.toml` and #3717\n\n<https://github.com/pj-tmt/tmt/issues/3717>".to_owned(),
                        "code" => format!("Before the code block.\n\n```rust\n  let recipient = member;\n{}\n```\n\nAfter the code block.", "a_long_identifier_".repeat(16)),
                        "quote" => "> A quiet quoted paragraph with **important words**.\n>\n> Another paragraph.\n\nNormal body text follows.".to_owned(),
                        "table" => "| Agent | State |\n|---|---|\n| alpha | **working** |\n| beta | waiting |".to_owned(),
                        _ => format!("| Agent | Notes |\n|---|---|\n| alpha | {} |", "a".repeat(180)),
                    };
                    app.view.as_mut().unwrap().replies[0]["response"] = json!(body);
                }
                draw(&app, width, 36);
                if scene == "expanded" {
                    let mut state = app.conversation.borrow_mut();
                    let active = state.active.clone().unwrap();
                    let session = state.sessions.get_mut(&active).unwrap();
                    assert!(session.folds.toggle(&super::super::body::Key::Outgoing(1)));
                    session.rendered = None;
                }
                let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
                terminal
                    .draw(|frame| render(frame, &app, frame.area()))
                    .unwrap();
                let buffer = terminal.backend().buffer();
                frames.push(json!({"scene":scene,"base":name,"depth":format!("{depth:?}"),"width":width,"height":36,
                    "lines":buffer.content.chunks(usize::from(width)).map(|row|row.iter().map(|cell|cell.symbol()).collect::<String>()).collect::<Vec<_>>(),
                    "cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>(),
                    "foldHits":app.conversation.borrow().fold_hits.len()}));
            }
        }
    }
    std::fs::write(output, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
}
