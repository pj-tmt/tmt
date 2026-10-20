//! `f` cycles the Agents tab's left chips: Squads, Agents (the flat attention list), Pinned.
use super::*;
use crate::board::{shell::OpsTab, workspace::AgentsFilter};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn f_cycles_squads_agents_pinned_and_the_agents_chip_is_the_flat_list() {
    let mut app = board();
    app.shell.tab = OpsTab::Agents;
    assert_eq!(app.workspace.filter, AgentsFilter::All);
    let tree = paint(&app, 160, 40).join("\n");
    assert!(tree.contains("Squads") && tree.contains("Pinned"), "{tree}");
    assert!(!app.flat_agents());
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.workspace.filter, AgentsFilter::Agents);
    assert!(app.flat_agents());
    let flat = paint(&app, 160, 40).join("\n");
    // The flat list has AGENT and SQUAD columns; the tree has one SQUAD / AGENT column.
    assert!(tree.contains("SQUAD / AGENT"), "{tree}");
    assert!(!flat.contains("SQUAD / AGENT"), "{flat}");
    assert!(flat.contains("AGENT") && flat.contains("SQUAD"), "{flat}");
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.workspace.filter, AgentsFilter::Pinned);
    assert!(!app.flat_agents());
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.workspace.filter, AgentsFilter::All);
}
