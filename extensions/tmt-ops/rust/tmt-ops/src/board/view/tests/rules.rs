//! Workspace boundaries use actual tree/conversation placement, not authored
//! factory panes. Frozen captures retain their previous provenance until review.
use super::*;
use crate::{
    board::{
        app::workspace::tests::{MEMBER, ROOM},
        shell::OpsTab,
        workspace::OpsTarget,
    },
    config::OpsLayout,
    test_support::stacked_rules,
};

const WIDTHS: [u16; 5] = [160, 140, 100, 80, 60];
const HEIGHTS: [u16; 7] = [8, 12, 13, 16, 18, 24, 31];

fn workspace(layout: OpsLayout, member: bool, tab: OpsTab) -> App {
    let mut app = workspace_board();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .layout = layout;
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":MEMBER,"requestId":"observed-response","submittedAtMs":1_000,
        "response":"First response line\nSecond response line\nThird response line"
    })];
    let target = if member {
        OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        }
    } else {
        OpsTarget::Squad(ROOM.into())
    };
    let index = app
        .workspace_rows()
        .iter()
        .position(|row| row.target == target)
        .unwrap();
    app.select(index);
    app.shell.tab = tab;
    app
}

#[test]
fn workspace_layouts_never_stack_boundary_rules_at_supported_sizes() {
    for tab in [OpsTab::Overview, OpsTab::Agents] {
        for layout in [OpsLayout::Auto, OpsLayout::Side, OpsLayout::Stack] {
            for member in [false, true] {
                let mut app = workspace(layout, member, tab);
                for width in WIDTHS {
                    app.set_body_width(width);
                    for height in HEIGHTS {
                        let lines = draw(&app, width, height);
                        if let Some(at) = stacked_rules(&lines) {
                            panic!(
                                "stacked rules at row {at}: {tab:?}/{layout:?} member={member} {width}x{height}\n{}",
                                lines.join("\n")
                            );
                        }
                    }
                }
            }
        }
    }
}

fn snapshots() -> Value {
    let mut frames = Vec::new();
    for layout in [OpsLayout::Auto, OpsLayout::Side, OpsLayout::Stack] {
        for member in [false, true] {
            for width in [160, 100, 80] {
                let mut app = workspace(layout, member, OpsTab::Agents);
                app.set_body_width(width);
                frames.push(json!({
                    "layout":format!("{layout:?}"),"selection":if member {"member"} else {"squad"},
                    "width":width,"height":24,"lines":draw(&app,width,24),
                }));
            }
        }
    }
    json!(frames)
}

#[test]
fn workspace_boundaries_snapshots_for_selected_squad_and_member() {
    assert_eq!(
        snapshots(),
        serde_json::from_str::<Value>(include_str!("rules_snapshots.json")).unwrap()
    );
}

#[test]
#[ignore = "explicit regeneration after decoded workspace-boundary approval"]
fn record_workspace_boundaries_snapshots() {
    std::fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/board/view/tests/rules_snapshots.json"
        ),
        serde_json::to_string_pretty(&snapshots()).unwrap() + "\n",
    )
    .unwrap();
}

/// Task-owned evidence only; frozen fixtures are never written by this capture.
#[test]
#[ignore = "read-only workspace-boundary inspection before fixture approval"]
fn dump_workspace_rule_snapshots() {
    let path = std::env::var("SQUAD_RULES_SNAPSHOTS_OUT").expect("task-owned output path");
    std::fs::write(
        path,
        serde_json::to_string_pretty(&snapshots()).unwrap() + "\n",
    )
    .unwrap();
}
