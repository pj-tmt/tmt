use super::*;
use crate::{
    board::workspace::{manage::Change, rename_member},
    membership::workspace::{Operation, RoomTarget},
};
use ratatui::buffer::Buffer;
use tmt_tui::components::scroll::Step;

fn draw(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}
fn label(buffer: &Buffer, area: Rect) -> String {
    (area.x..area.right())
        .map(|x| buffer[(x, area.y)].symbol())
        .collect()
}
fn review(kind: Kind, operation: Change) -> App {
    let mut app = board();
    assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
    let draft = app.workspace_form.as_mut().unwrap();
    draft.kind = kind;
    draft.operation = Some(operation);
    draft.values = vec!["typed text is not an action label".into()];
    app
}
fn rename_operation(expected_name: &str) -> Change {
    Change::RenameMember(rename_member::Request {
        identity_id: MEMBER.into(),
        expected_name: expected_name.into(),
        name: "worker-two".into(),
    })
}
#[test]
fn frozen_operation_names_the_whole_confirm_and_survives_changed_selection() {
    let room = RoomTarget {
        id: ROOM.into(),
        name: "product".into(),
        revision: 2,
    };
    let cases = [
        (
            Kind::Create,
            Change::Membership(Operation::Create {
                name: "new-squad".into(),
                lead: None,
                members: vec![],
            }),
            "Create squad",
        ),
        (
            Kind::Add,
            Change::Membership(Operation::Add {
                room: room.clone(),
                member: MEMBER.into(),
            }),
            "Add member",
        ),
        (
            Kind::Lead,
            Change::Membership(Operation::Lead {
                room: room.clone(),
                member: Some(MEMBER.into()),
            }),
            "Set lead",
        ),
        (
            Kind::Lead,
            Change::Membership(Operation::Lead {
                room: room.clone(),
                member: None,
            }),
            "Clear lead",
        ),
        (
            Kind::RenameSquad,
            Change::Membership(Operation::RenameSquad {
                room: room.clone(),
                name: "renamed".into(),
            }),
            "Rename",
        ),
        (
            Kind::Remove,
            Change::Membership(Operation::Remove {
                room: room.clone(),
                member: MEMBER.into(),
            }),
            "Remove membership",
        ),
        (
            Kind::Assign,
            Change::Membership(Operation::Assign {
                source: Some(room.clone()),
                destination: room.clone(),
                member: MEMBER.into(),
                move_member: false,
            }),
            "Add",
        ),
        (
            Kind::Assign,
            Change::Membership(Operation::Assign {
                source: Some(room.clone()),
                destination: room,
                member: MEMBER.into(),
                move_member: true,
            }),
            "Move",
        ),
        (Kind::RenameMember, rename_operation("worker"), "Rename"),
    ];
    for (kind, operation, expected) in cases {
        for width in [60, 100, 160] {
            let mut app = review(kind, operation.clone());
            assert_eq!(
                app.overlay_focus.focused().unwrap().last().unwrap(),
                "cancel"
            );
            let buffer = draw(&app, width, 36);
            let controls = app.workspace_form.as_ref().unwrap().controls.get();
            assert_eq!(label(&buffer, controls[0].unwrap()), " Cancel ");
            assert_eq!(
                label(&buffer, controls[1].unwrap()),
                format!(" {expected} ")
            );
            app.workspace.selected = Some(OpsTarget::Pool);
            assert_eq!(
                app.workspace_confirm(),
                Effect::Act(Request::WorkspaceOperation(operation.clone()))
            );
            assert_eq!(
                app.workspace_confirm(),
                Effect::None,
                "pending cannot replay"
            );
        }
    }
}
#[test]
fn short_review_fits_while_overflow_keeps_its_scroll_and_height_cap() {
    let app = review(Kind::RenameMember, rename_operation("worker"));
    draw(&app, 160, 36);
    let draft = app.workspace_form.as_ref().unwrap();
    let scroll = draft.scroll.borrow();
    assert_eq!(scroll.limit(), 0);
    assert_eq!(
        usize::from(app.input_band.get().unwrap().height),
        scroll.content() + 4
    );
    assert!(
        app.input_band.get().unwrap().height < 26,
        "no empty fixed-height body"
    );
    drop(scroll);
    let mut long = review(Kind::RenameMember, rename_operation("worker"));
    long.workspace_form.as_mut().unwrap().stage = Stage::Result(
        (0..80)
            .map(|i| format!("Retained result line {i}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    draw(&long, 160, 60);
    let draft = long.workspace_form.as_ref().unwrap();
    assert_eq!(long.input_band.get().unwrap().height, 26);
    assert_eq!(draft.scroll.borrow().content(), 80);
    assert!(draft.scroll.borrow().limit() > 0);
    assert!(draft.controls.get()[0].is_some());
    assert!(draft.controls.get()[1].is_none());
    draft.scroll.borrow_mut().step(Step::Bottom);
    let buffer = draw(&long, 160, 60);
    assert!(
        buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .contains("Retained result line 79")
    );
    assert_eq!(long.workspace_confirm(), Effect::None);
}
#[test]
fn hints_drop_whole_phrases_after_whole_controls_at_narrow_and_normal_widths() {
    for (width, expected) in [
        (24, ""),
        (60, "Esc cancel · Enter select"),
        (100, "Esc cancel · Enter select · Tab read/actions"),
        (160, "Esc cancel · Enter select · Tab read/actions"),
    ] {
        let app = review(Kind::RenameMember, rename_operation("worker"));
        let buffer = draw(&app, width, 36);
        let controls = app.workspace_form.as_ref().unwrap().controls.get();
        let last = controls
            .into_iter()
            .flatten()
            .max_by_key(|area| area.right())
            .unwrap();
        let outer = app.input_band.get().unwrap();
        let hint = Rect::new(
            last.right() + 1,
            last.y,
            outer.right().saturating_sub(2 + last.right() + 1),
            1,
        );
        assert_eq!(label(&buffer, hint).trim(), expected, "width {width}");
    }
}
#[test]
fn clipped_resized_and_scrolled_rename_cannot_gain_confirm_authority() {
    let mut app = review(Kind::RenameMember, rename_operation("worker"));
    draw(&app, 160, 36);
    assert!(app.workspace_form.as_ref().unwrap().controls.get()[1].is_some());
    app.invalidate_frame_geometry();
    assert_eq!(app.workspace_confirm(), Effect::None);
    draw(&app, 12, 8);
    assert_eq!(app.input_band.get().unwrap(), Rect::new(0, 0, 12, 8));
    assert!(app.workspace_form.as_ref().unwrap().controls.get()[1].is_none());
    assert_eq!(app.workspace_confirm(), Effect::None);
    draw(&app, 160, 36);
    assert!(matches!(
        app.workspace_confirm(),
        Effect::Act(Request::WorkspaceOperation(_))
    ));
    let mut long = review(
        Kind::RenameMember,
        rename_operation(&"long-name-".repeat(300)),
    );
    draw(&long, 100, 36);
    let draft = long.workspace_form.as_ref().unwrap();
    assert!(draft.scroll.borrow().limit() > 0);
    assert!(draft.controls.get()[1].is_none());
    draft.scroll.borrow_mut().step(Step::Bottom);
    draw(&long, 100, 36);
    assert!(
        long.workspace_form
            .as_ref()
            .unwrap()
            .scroll
            .borrow()
            .offset()
            > 0
    );
    assert!(long.workspace_form.as_ref().unwrap().controls.get()[1].is_none());
    assert_eq!(long.workspace_confirm(), Effect::None);
}
