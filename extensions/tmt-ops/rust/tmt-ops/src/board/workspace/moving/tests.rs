use super::*;
use crate::board::app::workspace::tests::{MEMBER, ROOM, board};
use crate::board::workspace::manage::Change;
use crate::board::workspace::{OpsMember, OpsObservation, OpsSquad};
use crate::membership::workspace::RoomTarget;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

const INFRA: &str = "77777777-7777-4777-8777-777777777777";
const SPARE: &str = "88888888-8888-4888-8888-888888888888";

/// `worker` in product, an empty infra squad, and `spare` in Common pool, all unfolded.
fn app() -> App {
    let mut app = board();
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.rows.insert(
        OpsTarget::Squad(INFRA.into()),
        json!({"name":"infra","roomId":INFRA,"revision":4}),
    );
    home.workspace.roster.squads.push(OpsSquad {
        room_id: INFRA.into(),
        name: "infra".into(),
        members: vec![],
        lead: None,
        member_states: Default::default(),
        observation: OpsObservation::Complete,
        idle_alert: None,
    });
    home.workspace.roster.members.insert(
        SPARE.into(),
        OpsMember {
            id: SPARE.into(),
            name: "spare".into(),
            state: Some("idle".into()),
            waiting: false,
            presence: Some(true),
            usage: None,
            activity: Default::default(),
        },
    );
    app.shell.tab = crate::board::shell::OpsTab::Agents;
    app.workspace.expanded.insert(ROOM.into());
    app.workspace.pool_expanded = true;
    app.workspace.selected = Some(worker());
    app
}

fn worker() -> OpsTarget {
    OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: MEMBER.into(),
    }
}

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn room(id: &str, name: &str, revision: u64) -> RoomTarget {
    RoomTarget {
        id: id.into(),
        name: name.into(),
        revision,
    }
}

fn applied(operation: Operation) -> Effect {
    Effect::Act(Request::WorkspaceOperation(Change::Membership(operation)))
}

fn lines(app: &App, width: u16) -> Vec<String> {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 36)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

#[test]
fn m_then_up_moves_a_squad_membership_to_the_squad_above() {
    let mut app = app();
    assert_eq!(press(&mut app, KeyCode::Char('m')), Effect::None);
    assert_eq!(app.moving.as_ref().unwrap().target.as_deref(), Some(ROOM));
    press(&mut app, KeyCode::Up);
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        applied(Operation::Assign {
            source: Some(room(ROOM, "product", 2)),
            destination: room(INFRA, "infra", 4),
            member: MEMBER.into(),
            move_member: true,
        })
    );
    assert!(app.moving.as_ref().unwrap().applying);
}

#[test]
fn common_pool_is_the_last_target_and_moving_there_leaves_the_squad() {
    let mut app = app();
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Down);
    assert_eq!(
        app.moving.as_ref().unwrap().target,
        None,
        "Common pool follows the squads (infra, product)"
    );
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        applied(Operation::Remove {
            room: room(ROOM, "product", 2),
            member: MEMBER.into(),
        })
    );
}

#[test]
fn a_pool_agent_joins_the_squad_it_is_moved_to() {
    let mut app = app();
    app.workspace.selected = Some(OpsTarget::Member {
        room_id: None,
        identity_id: SPARE.into(),
    });
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.moving.as_ref().unwrap().target, None);
    press(&mut app, KeyCode::Up);
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        applied(Operation::Assign {
            source: None,
            destination: room(ROOM, "product", 2),
            member: SPARE.into(),
            move_member: false,
        })
    );
}

#[test]
fn esc_puts_it_back_other_keys_are_ignored_and_the_same_squad_is_a_no_op() {
    let mut app = app();
    press(&mut app, KeyCode::Char('m'));
    for code in [
        KeyCode::Char('n'),
        KeyCode::Char('x'),
        KeyCode::Char('?'),
        KeyCode::Tab,
    ] {
        assert_eq!(press(&mut app, code), Effect::None);
        assert!(app.moving.is_some() && app.input.is_none() && !app.help);
    }
    assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.moving.is_none());
    assert_eq!(app.workspace.selected, Some(worker()));
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
    assert!(app.moving.is_none());
    assert_eq!(app.notice.as_deref(), Some("worker stays in product."));
}

#[test]
fn the_result_is_one_status_line_and_a_refused_move_changes_nothing() {
    let mut app = app();
    let rows = |app: &App| -> Vec<_> {
        app.workspace_rows()
            .iter()
            .map(|row| (row.target.clone(), row.title.clone()))
            .collect()
    };
    let before = rows(&app);
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    app.finished_workspace(Err(crate::core::SquadError::new(
        "ROOM_REVISION_CONFLICT",
        "The room changed",
    )));
    assert_eq!(app.notice.as_deref(), Some("Not applied: The room changed"));
    assert!(app.moving.is_none());
    assert_eq!(rows(&app), before);
    press(&mut app, KeyCode::Char('m'));
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    app.finished_workspace(Ok(crate::membership::Outcome {
        document: json!({}),
        complete: true,
    }));
    assert_eq!(app.notice.as_deref(), Some("Moved worker to infra."));
}

#[test]
fn the_list_paints_the_lifted_line_and_its_old_place_at_every_width() {
    for width in [160, 100, 80] {
        let mut app = app();
        press(&mut app, KeyCode::Char('m'));
        let shown = lines(&app, width).join("\n");
        assert!(shown.contains("⇅ worker  → product"), "{width}: {shown}");
        assert!(!shown.contains("· moving"), "{width}: {shown}");
        press(&mut app, KeyCode::Up);
        let shown = lines(&app, width);
        let lifted = shown
            .iter()
            .position(|line| line.contains("⇅ worker  → infra"))
            .unwrap_or_else(|| panic!("{width}: {shown:#?}"));
        let infra = shown
            .iter()
            .position(|line| line.contains("infra"))
            .unwrap_or_else(|| panic!("{width}: {shown:#?}"));
        // The lifted line stays in view; a short stacked list may scroll its old place out.
        let placeholder = shown
            .iter()
            .position(|line| line.contains("┆ worker · moving"));
        assert!(placeholder.is_some() || width < 160, "{width}: {shown:#?}");
        assert!(infra < lifted, "{width}: {shown:#?}");
        assert!(
            placeholder.is_none_or(|at| lifted < at),
            "{width}: {shown:#?}"
        );
        assert!(
            shown
                .iter()
                .any(|line| line.contains("Moving worker · ↑↓ choose a squad")),
            "{width}"
        );
        assert!(
            shown
                .last()
                .unwrap()
                .contains("↑↓ choose squad  ⏎ move to infra  Esc cancel"),
            "{width}: {:?}",
            shown.last()
        );
    }
}
