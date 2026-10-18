//! Explicit whole-App evidence; never writes frozen fixtures.
use crate::board::{shell::OpsTab, workspace::OpsTarget};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

#[test]
#[ignore = "explicit top-bar look packet"]
fn capture_top_bar() {
    let output = std::env::var("TMT_TOP_BAR_CAPTURE").expect("task-owned capture path");
    let now = 1_791_600_000_000;
    crate::status::with_now_ms(now, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut frames = Vec::new();
        for case in ["known", "used-only-zero", "unknown", "unselected"] {
            for tab in [OpsTab::Overview, OpsTab::Agents, OpsTab::Digest] {
                for width in [148, 100, 60] {
                    for (base, theme) in [
                        ("tmt", tmt_cli_style::Base::Tmt),
                        ("tmt-light", tmt_cli_style::Base::TmtLight),
                    ] {
                        let (mut app, members, _) = super::host_tests::observed_app(now);
                        let look = crate::look::Look {
                            theme: tmt_cli_style::Theme::new(theme),
                            depth: tmt_cli_style::Depth::TrueColor,
                        };
                        let view = app.view.as_mut().unwrap();
                        view.look = look;
                        view.home.as_mut().unwrap().workspace.squad_looks.clear();
                        app.shell.tab = tab;
                        app.workspace.selected = Some(OpsTarget::Member {
                            room_id: Some(
                                app.workspace_snapshot().unwrap().roster.squads[0]
                                    .room_id
                                    .clone(),
                            ),
                            identity_id: members[usize::from(case == "used-only-zero")].clone(),
                        });
                        if case == "unknown" {
                            app.references
                                .agents
                                .values_mut()
                                .for_each(|agent| agent.context = None);
                            app.limits
                                .as_mut()
                                .unwrap()
                                .iter_mut()
                                .for_each(|standing| standing.latest = None);
                        } else if case == "unselected" {
                            app.workspace.selected = None;
                        }
                        assert_eq!(app.look().depth, look.depth);
                        assert_eq!(
                            app.look().role(tmt_cli_style::Role::Text),
                            look.role(tmt_cli_style::Role::Text)
                        );
                        app.set_body_width(width);
                        let mut terminal = Terminal::new(TestBackend::new(width, 44)).unwrap();
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                        let buffer = terminal.backend().buffer();
                        let cells: Vec<_> = buffer.content.iter().map(|cell| json!({
                            "symbol":cell.symbol(), "fg":format!("{:?}",cell.fg),
                            "bg":format!("{:?}",cell.bg), "modifier":format!("{:?}",cell.modifier)
                        })).collect();
                        let lines: Vec<_> = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                            .collect();
                        let surface = tab.label().to_ascii_lowercase();
                        frames.push(json!({
                            "scene":format!("top-bar-{case}"), "surface":surface,"requestedSurface":surface,
                            "width":width,"height":44,"base":base,"depth":"TrueColor",
                            "cells":cells,"lines":lines,"hits":format!("{:?}",app.hits.borrow()),
                            "selected":format!("{:?}",app.workspace.selected)
                        }));
                    }
                }
            }
        }
        assert_eq!(frames.len(), 72);
        std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
    });
}
