//! Explicit renderer evidence; never writes a checked-in fixture.
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

#[test]
#[ignore = "explicit panel-policy renderer packet"]
fn capture_panel_policy_packet() {
    let output = std::env::var("TMT_PANELS_CAPTURE").expect("task-owned capture path");
    let _no_commands = crate::runner::forbid_commands();
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut frames = Vec::new();
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            for scene in ["default", "picker", "edited", "reset", "saving", "failed"] {
                for width in [160, 100, 60] {
                    for (base, depth) in [
                        ("tmt", tmt_cli_style::Depth::TrueColor),
                        ("tmt-light", tmt_cli_style::Depth::TrueColor),
                        ("tmt", tmt_cli_style::Depth::None),
                    ] {
                        let look = crate::look::Look {
                            theme: tmt_cli_style::Theme::new(
                                tmt_cli_style::Base::parse(base).unwrap(),
                            ),
                            depth,
                        };
                        let mut app =
                            crate::board::app::workspace::tests::capture_case("overview", look);
                        app.shell.tab = tab;
                        if scene != "default" {
                            app.open_panels();
                            app.panels_event(&key(KeyCode::Enter));
                            app.panels_event(&key(KeyCode::Down));
                            app.panels_event(&key(KeyCode::Char('u')));
                            if scene == "edited" {
                                let next = app.panels_picker.as_ref().unwrap().next();
                                app.panels_picker = None;
                                app.view
                                    .as_mut()
                                    .unwrap()
                                    .home
                                    .as_mut()
                                    .unwrap()
                                    .workspace
                                    .preferences = next;
                            } else if scene == "reset" {
                                app.panels_event(&key(KeyCode::End));
                                app.panels_event(&key(KeyCode::Up));
                                app.panels_event(&key(KeyCode::Up));
                                app.panels_event(&key(KeyCode::Enter));
                            } else if matches!(scene, "saving" | "failed") {
                                app.panels_event(&key(KeyCode::End));
                                app.panels_event(&key(KeyCode::Up));
                                assert!(matches!(
                                    app.panels_event(&key(KeyCode::Enter)),
                                    Some(Effect::Act(_))
                                ));
                                if scene == "failed" {
                                    app.panels_finished(Err(
                                        "Workspace preferences changed; reload before saving."
                                            .into(),
                                    ));
                                }
                            }
                        }
                        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
                        terminal
                            .draw(|frame| {
                                crate::board::view::prepare(&mut app, frame.area());
                                crate::board::view::render(frame, &app);
                            })
                            .unwrap();
                        let buffer = terminal.backend().buffer();
                        let cells: Vec<_> = buffer.content.iter().map(|cell| json!({
                            "symbol":cell.symbol(), "fg":format!("{:?}",cell.fg),
                            "bg":format!("{:?}",cell.bg), "modifier":format!("{:?}",cell.modifier)
                        })).collect();
                        let lines: Vec<String> = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                            .collect();
                        frames.push(json!({"scene":format!("{tab:?}-{scene}"), "width":width,
                            "height":36,"base":base,"depth":format!("{depth:?}"), "cells":cells,"lines":lines}));
                    }
                }
            }
        }
        std::fs::write(output, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
    });
}
