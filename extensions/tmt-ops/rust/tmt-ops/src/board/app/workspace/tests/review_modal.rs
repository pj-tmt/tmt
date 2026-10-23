use super::*;
use crate::{
    board::workspace::{manage::Change, rename_member},
    membership::workspace::{Operation, RoomTarget},
};
use ratatui::buffer::Buffer;
use tmt_tui::components::Step;

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
            "Leave squad",
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
        let mut cancel = review(kind, operation.clone());
        draw(&cancel, 160, 36);
        assert_eq!(key(&mut cancel, KeyCode::Enter), Effect::None);
        assert!(
            cancel.workspace_form.is_none(),
            "initial Enter cancels {expected}"
        );
        for width in [60, 100, 160] {
            let mut app = review(kind, operation.clone());
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
fn management_review_renders_the_frame_title_once() {
    let app = review(
        Kind::Remove,
        Change::Membership(Operation::Remove {
            room: RoomTarget {
                id: ROOM.into(),
                name: "product".into(),
                revision: 2,
            },
            member: MEMBER.into(),
        }),
    );
    for (width, height) in [(160, 36), (100, 24), (60, 8)] {
        let buffer = draw(&app, width, height);
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert_eq!(
            text.matches("Leave squad · product").count(),
            1,
            "{width}x{height}: {text}"
        );
        assert!(text.contains("Remove membership: worker"));
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
        (60, "c copy command · Esc cancel"),
        (100, "c copy command · Esc cancel · Enter select"),
        (160, "c copy command · Esc cancel · Enter select"),
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

/// Task-owned decoded review evidence, never a frozen-fixture recorder.
#[test]
#[ignore = "explicit fitted review matrix; requires TMT_OVERLAY_CUE_OUTPUT"]
fn capture_fitted_management_reviews() {
    let directory = std::path::PathBuf::from(
        std::env::var("TMT_OVERLAY_CUE_OUTPUT").expect("task-owned output directory"),
    );
    std::fs::create_dir_all(&directory).unwrap();
    crate::status::with_now_ms(1_791_600_000_000, || {
        for scene in ["rename-review", "membership-review"] {
            for (theme, look) in crate::board::picker_surface::evidence::looks()
                .into_iter()
                .enumerate()
            {
                for width in [160, 100, 60] {
                    for height in [36, 24, 8] {
                        let mut app = if scene == "rename-review" {
                            review(Kind::RenameMember, rename_operation("worker"))
                        } else {
                            review(
                                Kind::Remove,
                                Change::Membership(Operation::Remove {
                                    room: RoomTarget {
                                        id: ROOM.into(),
                                        name: "product".into(),
                                        revision: 2,
                                    },
                                    member: MEMBER.into(),
                                }),
                            )
                        };
                        app.view.as_mut().unwrap().look = look;
                        let buffer = draw(&app, width, height);
                        let draft = app.workspace_form.as_ref().unwrap();
                        let scroll = draft.scroll.borrow();
                        let cells = buffer.content.iter().map(|c| json!({"symbol":c.symbol(), "fg":format!("{:?}", c.fg), "bg":format!("{:?}", c.bg), "modifier":format!("{:?}", c.modifier)})).collect::<Vec<_>>();
                        let lines = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                            .collect::<Vec<_>>();
                        let rect = |r: Rect| [r.x, r.y, r.width, r.height];
                        let value = json!({"scene":scene,"theme":theme,"width":width,"height":height,"cells":cells,"lines":lines,"controls":draft.controls.get().map(|r|r.map(rect)),"inputBand":app.input_band.get().map(rect),"scrollOffset":scroll.offset(),"scrollContent":scroll.content(),"scrollViewport":rect(scroll.viewport()),"selected":format!("{:?}",app.workspace.selected),"focus":format!("{:?}",app.overlay_focus.focused()),"operation":format!("{:?}",draft.operation)});
                        let name = format!("management-{scene}-{theme}-{width}x{height}.json");
                        std::fs::write(directory.join(name), serde_json::to_vec(&value).unwrap())
                            .unwrap();
                    }
                }
            }
        }
    });
}

#[test]
fn crud_keys_keep_create_uppercase_and_remove_distinct_from_leave() {
    let mut app = board();
    assert!(!app.bindings().contains_key("n"));
    assert_eq!(app.bindings()["N"].text, "create-squad");
    assert_eq!(app.bindings()["d"].text, "remove-member");
    assert_eq!(app.bindings()["x"].text, "remove-membership");
    for target in [OpsTarget::Squad(ROOM.into()), OpsTarget::Pool] {
        app.workspace.selected = Some(target);
        assert_eq!(app.bindings()["N"].text, "create-squad");
        assert!(!app.bindings().contains_key("n"));
        assert!(!app.bindings().contains_key("d"));
    }
}

#[test]
fn remove_member_is_copy_only_cancel_default_and_frozen_to_the_displayed_name() {
    let mut app = board();
    key(&mut app, KeyCode::Char('d'));
    assert!(app.workspace_form.is_some(), "d opens remove member");
    for width in [80, 150] {
        let text = paint(&app, width, 43).join("\n");
        assert!(text.contains("Remove member"));
        assert!(text.contains("Stop the agent first"));
        assert!(text.contains("tmt rm --force -- worker"));
        assert!(text.contains("Copy command"));
        assert!(!text.contains("'stop'"));
    }
    app.workspace.selected = Some(OpsTarget::Pool);
    let Effect::Act(Request::Copy { text, .. }) = key(&mut app, KeyCode::Char('c')) else {
        panic!("copy only; no workspace operation or foreground exec")
    };
    assert!(text.ends_with("tmt rm --force -- worker"));
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(app.workspace_form.is_none(), "initial Enter cancels");
}

#[test]
fn removal_quoting_is_literal_and_never_leaks_into_other_scopes() {
    let mut app = board();
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
        .unwrap()["name"] = json!("worker's-name");
    key(&mut app, KeyCode::Char('d'));
    paint(&app, 80, 43);
    let Effect::Act(Request::Copy { text, .. }) = key(&mut app, KeyCode::Char('c')) else {
        panic!("copy");
    };
    assert!(text.ends_with("'worker'\\''s-name'"));
    key(&mut app, KeyCode::Esc);
    assert!(app.workspace_form.is_none());
    app.shell.tab = OpsTab::Digest;
    assert!(!app.bindings().values().any(|a| a.text == "remove-member"));
    app.workspace_begin(Kind::RemoveMember);
    assert!(app.workspace_form.is_none());
    app.shell.tab = OpsTab::Agents;
    app.focus = 1;
    assert!(!app.bindings().values().any(|a| a.text == "remove-member"));
}

#[test]
fn crud_reviews_wrap_with_body_margins_and_keep_clipped_mutations_disabled() {
    for scene in ["crud-rename-review", "crud-move-review", "crud-remove"] {
        for width in [80, 150] {
            let mut app = crud_capture_case(scene, crate::look::Look::default());
            let text = paint(&app, width, 43).join("\n");
            let area = app.input_band.get().unwrap();
            assert_eq!(area.width, 80.min(width - 4));
            assert!(area.x >= 2 && area.right() <= width - 2);
            assert!(text.contains("Cancel"));
            if scene == "crud-rename-review" {
                assert!(
                    app.workspace_form
                        .as_ref()
                        .unwrap()
                        .review()
                        .contains("tmt mv worker worker-two")
                );
            }
            app.invalidate_frame_geometry();
            if scene != "crud-remove" {
                assert_eq!(app.workspace_confirm(), Effect::None);
            }
        }
    }
}

pub(in crate::board) const CRUD_SCENES: &[&str] = &[
    "crud-member",
    "crud-squad",
    "crud-pool",
    "crud-rename-fields",
    "crud-rename-review",
    "crud-rename-pending",
    "crud-rename-success",
    "crud-rename-conflict",
    "crud-move-fields",
    "crud-move-review",
    "crud-move-success",
    "crud-move-refused",
    "crud-move-partial",
    "crud-remove",
    "crud-remove-copied",
    "crud-leave-squad",
    "crud-remove-long",
];
pub(in crate::board) fn crud_capture_case(scene: &str, look: crate::look::Look) -> App {
    let mut app = board();
    app.view.as_mut().unwrap().look = look;
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    match scene {
        "crud-member" => {}
        "crud-squad" => app.workspace.selected = Some(OpsTarget::Squad(ROOM.into())),
        "crud-pool" => app.workspace.selected = Some(OpsTarget::Pool),
        "crud-remove" | "crud-remove-copied" | "crud-remove-long" => {
            app.workspace_begin(Kind::RemoveMember);
            if scene == "crud-remove-copied" {
                app.set_notice(
                    Some(crate::effects::Copied::Clipboard.describe().into()),
                    false,
                );
            }
            if scene == "crud-remove-long" {
                app.workspace_form
                    .as_mut()
                    .unwrap()
                    .member
                    .as_mut()
                    .unwrap()
                    .name = "worker-with-a-long-name-and-a-'quoted'-suffix".into();
            }
        }
        "crud-leave-squad" => {
            app.workspace_begin(Kind::Remove);
        }
        scene if scene.starts_with("crud-rename-") => {
            app.workspace_begin(Kind::RenameMember);
            if scene != "crud-rename-fields" {
                app.input = None;
                app.workspace_submit_field("worker-two".into());
                let draft = app.workspace_form.as_mut().unwrap();
                if scene != "crud-rename-review" {
                    draft.stage = Stage::Pending;
                }
                if matches!(scene, "crud-rename-success" | "crud-rename-conflict") {
                    let Change::RenameMember(request) = draft.operation.clone().unwrap() else {
                        unreachable!()
                    };
                    app.finished_member_rename(rename_member::Completed {
                        request,
                        outcome: Ok(if scene == "crud-rename-success" {
                            rename_member::Outcome::Applied {
                                name: "worker-two".into(),
                                changed: true,
                            }
                        } else {
                            rename_member::Outcome::Conflict {
                                name: "competitor".into(),
                            }
                        }),
                    });
                }
            }
        }
        scene if scene.starts_with("crud-move-") => {
            app.workspace_begin(Kind::Assign);
            if scene != "crud-move-fields" {
                app.input = None;
                let draft = app.workspace_form.as_mut().unwrap();
                draft.operation = Some(Change::Membership(Operation::Assign {
                    source: Some(RoomTarget {
                        id: ROOM.into(),
                        name: "product".into(),
                        revision: 2,
                    }),
                    destination: RoomTarget {
                        id: "33333333-3333-4333-8333-333333333333".into(),
                        name: "platform".into(),
                        revision: 3,
                    },
                    member: MEMBER.into(),
                    move_member: true,
                }));
                draft.stage = Stage::Review;
                match scene {
                    "crud-move-success" => app.finished_workspace(Ok(crate::membership::Outcome {
                        document: json!({"source":"product","destination":"platform","member":{"id":MEMBER,"name":"worker"},"pausedTimers":0,"sourceRemoval":{"removed":true}}), complete: true,
                    })),
                    "crud-move-refused" => app.finished_workspace(Err(crate::core::SquadError::new("ROOM_REVISION_CONFLICT", "The source squad revision changed"))),
                    "crud-move-partial" => app.finished_workspace(Ok(crate::membership::Outcome {
                        document: json!({"failed":{"error":{"message":"Source removal was not confirmed"}},"results":[{"name":"destination addition"},{"name":"source removal","error":{"code":"CORE_UNAVAILABLE","message":"Source removal was not confirmed"}}]}), complete: false,
                    })),
                    _ => {}
                }
            }
        }
        _ => panic!("unknown CRUD scene {scene}"),
    }
    app
}

#[test]
fn member_footer_uses_effective_crud_bindings_and_never_advertises_future_actions() {
    let mut app = board();
    let text = paint(&app, 150, 43).join("\n");
    for hint in [
        "r rename",
        "m move",
        "R resume",
        "d remove",
        "x leave squad",
    ] {
        assert!(text.contains(hint), "{hint}: {text}");
    }
    assert!(!text.contains("n new"));
    assert!(!text.contains("S switch tool"));
    app.shell.tab = OpsTab::Digest;
    let digest = paint(&app, 150, 43).join("\n");
    assert!(!digest.contains("d remove"));
    let app = configured_member(
        "[bind]\nd = \"help\"\nz = \"remove-member\"\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    let text = paint(&app, 150, 43).join("\n");
    assert!(text.contains("z remove"));
    assert!(!text.contains("d remove"));
}

#[test]
fn rename_review_copies_the_human_equivalent_without_changing_the_cas() {
    let mut app = crud_capture_case("crud-rename-review", crate::look::Look::default());
    let draft = app.workspace_form.as_ref().unwrap();
    let before = draft.operation.clone();
    let text = draft.review();
    assert!(text.contains("tmt mv worker worker-two"));
    assert!(text.contains("Stops if the name changed first."));
    assert!(!text.contains("printf"));
    assert!(!text.contains("identity.rename"));
    paint(&app, 80, 43);
    let Effect::Act(Request::Copy { text, .. }) = key(&mut app, KeyCode::Char('c')) else {
        panic!("copy equivalent command");
    };
    assert_eq!(text, "tmt mv worker worker-two");
    assert_eq!(app.workspace_form.as_ref().unwrap().operation, before);
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
    let Effect::Act(Request::WorkspaceOperation(operation)) = app.workspace_confirm() else {
        panic!("CAS remains the executable operation");
    };
    assert_eq!(Some(operation), before);
}

#[test]
fn move_review_is_human_sized_with_internal_ids_only_in_the_commands() {
    let app = crud_capture_case("crud-move-review", crate::look::Look::default());
    let draft = app.workspace_form.as_ref().unwrap();
    assert_eq!(draft.title(), "Move to squad");
    assert_eq!(
        draft.review(),
        format!(
            "worker: product → platform\n\ntmt ops sq add --squad platform -- {MEMBER}\ntmt ops sq rm --squad product -- {MEMBER}\n\nHistory, drafts and timers stay; source timers pause until resumed."
        )
    );
}

#[test]
fn rename_command_quotes_only_shell_sensitive_values_and_protects_option_operands() {
    for (old, new, expected) in [
        ("worker", "worker-two", "tmt mv worker worker-two"),
        (
            "worker's-name",
            "new name",
            "tmt mv 'worker'\\''s-name' 'new name'",
        ),
        ("-worker", "new", "tmt mv -- -worker new"),
        (
            "worker",
            "$(touch marker)",
            "tmt mv worker '$(touch marker)'",
        ),
    ] {
        let mut app = review(Kind::RenameMember, rename_operation(old));
        let draft = app.workspace_form.as_mut().unwrap();
        let Some(Change::RenameMember(request)) = draft.operation.as_mut() else {
            unreachable!()
        };
        request.name = new.into();
        assert_eq!(draft.review_command().as_deref(), Some(expected));
    }
}
