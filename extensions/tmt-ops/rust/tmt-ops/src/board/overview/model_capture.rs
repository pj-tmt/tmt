//! Explicit whole-App evidence for the dense model columns and lead marker.
use crate::board::{app::workspace::tests::populated_board, inventory::Inventory, shell::OpsTab};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

#[test]
#[ignore = "explicit model-column look packet; never records frozen fixtures"]
fn capture_model_columns() {
    let output = std::env::var("TMT_MODEL_COLUMNS_CAPTURE").expect("task-owned capture path");
    crate::status::with_now_ms(1_791_600_000_000, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut frames = Vec::new();
        for tab in [OpsTab::Agents] {
            for width in [160, 100, 60] {
                for (base, theme) in [
                    ("tmt", tmt_cli_style::Base::Tmt),
                    ("tmt-light", tmt_cli_style::Base::TmtLight),
                ] {
                    let look = crate::look::Look {
                        theme: tmt_cli_style::Theme::new(theme),
                        depth: tmt_cli_style::Depth::TrueColor,
                    };
                    let mut app = populated_board(2, 4);
                    app.shell.tab = tab;
                    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
                    let view = app.view.as_mut().unwrap();
                    view.look = look;
                    let workspace = &mut view.home.as_mut().unwrap().workspace;
                    workspace.squad_looks.clear();
                    let mut listed = Vec::new();
                    for (ordinal, member) in workspace.roster.members.values().enumerate() {
                        let (driver, start, observed) = match ordinal {
                            0 => (Some("claude"), "claude-sonnet-5-5", Some("claude-opus-5-5")),
                            1 => (Some("claude"), "claude-sonnet-5-5", None),
                            2 => (Some("codex"), "gpt-6.1-sol", None),
                            _ => (None, "claude-stale-model", None),
                        };
                        listed.push(json!({
                            "id":member.id, "name":member.name, "runningDriver":driver,
                            "resume":{"model":start,"consumption":{"modelId":observed}}
                        }));
                    }
                    let first = workspace.roster.squads[0].members[0].clone();
                    workspace.preferences.agent_pins.insert(first.clone());
                    app.workspace.selected = Some(crate::board::workspace::OpsTarget::Member {
                        room_id: Some(workspace.roster.squads[0].room_id.clone()),
                        identity_id: first,
                    });
                    app.references = Inventory::from_listed(&[], &Ok(json!({"identities":listed})));
                    app.workspace_reconcile();
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
                        "symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),
                        "bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)
                    })).collect();
                    let lines: Vec<_> = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect();
                    let surface = if tab == OpsTab::Agents {
                        "overview"
                    } else {
                        "agents"
                    };
                    frames.push(json!({
                        "scene":"model-columns","surface":surface,"requestedSurface":surface,
                        "width":width,"height":44,"base":base,"depth":"TrueColor",
                        "cells":cells,"lines":lines,"hits":format!("{:?}",app.hits.borrow()),
                        "selected":format!("{:?}",app.workspace.selected)
                    }));
                }
            }
        }
        assert_eq!(frames.len(), 12);
        std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
    });
}
