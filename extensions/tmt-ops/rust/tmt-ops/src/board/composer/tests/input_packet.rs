//! Read-only full-frame evidence for the shared input at responsive boundaries.
use super::*;
use crate::board::{app::workspace::tests::MEMBER, inventory::Inventory, shell::OpsTab};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tmt_cli_style::{Base, Depth, Theme};

#[test]
#[ignore = "exports input look scenes only to an explicitly supplied task-owned path"]
fn capture_conversation_input_packet() {
    let output = std::env::var_os("TMT_INPUT_PACKET").expect("task-owned packet path");
    let _no_commands = crate::runner::forbid_commands();
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut frames = Vec::new();
        for (name, base, depth) in [
            ("tmt", Base::Tmt, Depth::TrueColor),
            ("tmt-light", Base::TmtLight, Depth::TrueColor),
            ("terminal", Base::Terminal, Depth::Ansi16),
            ("NO_COLOR", Base::Tmt, Depth::None),
        ] {
            for tab in [OpsTab::Agents, OpsTab::Overview] {
                for width in [160, 100, 60] {
                    for scene in [
                        "idle",
                        "focused-empty",
                        "one-line",
                        "three-lines",
                        "five-lines",
                        "note",
                        "unknown-window",
                    ] {
                        let mut app = crate::board::app::workspace::tests::board();
                        app.shell.tab = tab;
                        if tab == OpsTab::Overview && width < 140 {
                            app.focus = 1; // Enter the existing compact reader path before editing.
                        }
                        let look = crate::look::Look {
                            theme: Theme::new(base),
                            depth,
                        };
                        let view = app.view.as_mut().unwrap();
                        view.look = look;
                        view.home.as_mut().unwrap().workspace.squad_looks.clear();
                        let selected = app.workspace.selected.clone().unwrap();
                        app.view
                            .as_mut()
                            .unwrap()
                            .home
                            .as_mut()
                            .unwrap()
                            .workspace
                            .rows
                            .get_mut(&selected)
                            .unwrap()["waitingOnYou"] = json!([]);
                        app.references = Inventory::from_listed(
                            &[],
                            &Ok(json!({"identities":[{
                                "id":MEMBER,"name":"worker","runningDriver":"codex",
                                "resume":{"usage":{"tokens":142000,"observedAtMs":1_791_600_000_000u64,
                                    "windowTokens":if scene == "unknown-window" {None} else {Some(258000)}}}
                            }]})),
                        );
                        draw(&mut app, width, 36);
                        if scene == "note" {
                            // This literal observed squad names the selected agent as its lead.
                            app.view
                                .as_mut()
                                .unwrap()
                                .home
                                .as_mut()
                                .unwrap()
                                .workspace
                                .roster
                                .squads[0]
                                .lead = Some(MEMBER.into());
                            app.perform(&crate::action::Action::parse("annotate lead").unwrap());
                            assert!(app.input.is_some(), "the observed lead note is admitted");
                        } else if !matches!(scene, "idle" | "unknown-window") {
                            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                            assert!(app.input.is_some());
                        }
                        let text = match scene {
                            "one-line" => "One message to the selected member.",
                            "three-lines" => "First line\nSecond line\nThird line",
                            "five-lines" => {
                                "First line\nSecond line\nThird line\nFourth line\nFifth line"
                            }
                            _ => "",
                        };
                        if !text.is_empty() {
                            app.paste(text);
                        }
                        let buffer = draw(&mut app, width, 36);
                        assert_eq!(
                            app.look(),
                            look,
                            "the actual resolved look matches the label"
                        );
                        assert_eq!(
                            app.shell.tab,
                            if tab == OpsTab::Overview && width < 140 {
                                OpsTab::Agents
                            } else {
                                tab
                            },
                            "compact reader entry uses the existing tab owner"
                        );
                        let band = app.input_band.get().expect("visible input band");
                        let editor = app.input_viewport.get().expect("visible editor").0;
                        assert_eq!(editor.bottom(), band.bottom() - 1);
                        frames.push(json!({"scene":scene,"requestedSurface":tab.label(),"surface":app.shell.tab.label(),"base":name,
                            "depth":format!("{depth:?}"),"width":width,"height":36,
                            "lines":buffer.content.chunks(usize::from(width)).map(|row|row.iter().map(|cell|cell.symbol()).collect::<String>()).collect::<Vec<_>>(),
                            "cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>(),
                            "input":format!("{band:?}"),"editor":format!("{editor:?}")}));
                    }
                }
            }
        }
        std::fs::write(output, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
    });
}
