use super::*;
use crate::{
    board::workspace::{manage::Change, rename_member},
    membership::workspace::{Operation, RoomTarget},
};
use ratatui::buffer::Buffer;
use tmt_tui::components::Step;

#[test]
fn new_member_opens_before_the_host_template_read_on_every_agents_row() {
    for target in [
        OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        },
        OpsTarget::Squad(ROOM.into()),
        OpsTarget::Pool,
    ] {
        let mut app = board();
        app.workspace.selected = Some(target.clone());
        app.workspace_reconcile();
        let _no_commands = crate::runner::forbid_commands();
        key(&mut app, KeyCode::Char('n'));
        let screen = paint(&app, 150, 43).join("\n");
        assert!(screen.contains("New member"), "{target:?}: {screen}");
        assert!(screen.contains("Loading"), "{target:?}: {screen}");
        assert!(!screen.contains("Squad name"), "n never creates a squad");
    }
}

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
        (Kind::RenameMember, rename_operation("worker"), "Rename"),
    ];
    for (kind, operation, expected) in cases {
        let mut cancel = review(kind, operation.clone());
        draw(&cancel, 160, 36);
        assert_eq!(key(&mut cancel, KeyCode::Enter), Effect::None);
        assert!(hidden(&cancel), "initial Enter cancels {expected}");
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
    let buffer = draw(&app, 160, 36);
    let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(
        !text.contains("Read ·"),
        "a fitted body has no read-position line"
    );
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
    assert!(
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .contains("Read ·"),
        "overflow retains its position line"
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
    assert_eq!(app.bindings()["n"].text, "new-member");
    assert_eq!(app.bindings()["C"].text, "member-checklist");
    assert!(!app.bindings().contains_key("N"));
    assert!(!app.bindings().contains_key("d"));
    assert_eq!(app.bindings()["x"].text, "remove-member");
    for target in [OpsTarget::Squad(ROOM.into()), OpsTarget::Pool] {
        app.workspace.selected = Some(target);
        assert_eq!(app.bindings()["N"].text, "create-squad");
        assert!(!app.bindings().contains_key("C"));
        assert_eq!(app.bindings()["n"].text, "new-member");
        assert!(!app.bindings().contains_key("d"));
    }
}

#[test]
fn close_member_is_cancel_default_and_frozen_to_the_displayed_name() {
    let mut app = board();
    member_menu_action(&mut app, OpsUiAction::RemoveMember);
    for width in [80, 150] {
        let text = paint(&app, width, 43).join("\n");
        assert!(text.contains("Close and delete"));
        assert!(text.contains("Delete identity worker"));
        assert!(text.contains("Conversation history and notes are kept"));
        assert!(!text.contains("Copy command"));
        assert!(!text.contains("tmt rm"));
    }
    app.workspace.selected = Some(OpsTarget::Pool);
    assert_eq!(
        app.workspace_form
            .as_ref()
            .unwrap()
            .member
            .as_ref()
            .unwrap()
            .name,
        "worker"
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('c')),
        Effect::None,
        "no copy-only lifecycle command"
    );
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(hidden(&app), "initial Enter cancels");
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
    member_menu_action(&mut app, OpsUiAction::RemoveMember);
    paint(&app, 80, 43);
    assert_eq!(
        app.workspace_form
            .as_ref()
            .unwrap()
            .member
            .as_ref()
            .unwrap()
            .name,
        "worker's-name"
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('c')),
        Effect::None,
        "literal name never becomes a shell command"
    );
    key(&mut app, KeyCode::Esc);
    assert!(hidden(&app));
    app.shell.tab = OpsTab::Digest;
    assert!(!app.bindings().values().any(|a| a.text == "remove-member"));
    app.workspace_begin(Kind::RemoveMember);
    assert!(
        hidden(&app),
        "the kept draft does not reopen outside the Agents list"
    );
    app.shell.tab = OpsTab::Agents;
    app.focus = 1;
    assert!(!app.bindings().values().any(|a| a.text == "remove-member"));
}

#[test]
fn rename_review_wraps_with_body_margins_and_keeps_clipped_mutations_disabled() {
    for width in [80, 150] {
        let mut app = crud_capture_case("crud-rename-review", crate::look::Look::default());
        let text = paint(&app, width, 43).join("\n");
        let area = app.input_band.get().unwrap();
        assert_eq!(area.width, 80.min(width - 4));
        assert!(area.x >= 2 && area.right() <= width - 2);
        assert!(text.contains("Cancel"));
        assert!(
            app.workspace_form
                .as_ref()
                .unwrap()
                .review()
                .contains("tmt mv worker worker-two")
        );
        app.invalidate_frame_geometry();
        assert_eq!(app.workspace_confirm(), Effect::None);
    }
}

pub(in crate::board) const CRUD_SCENES: &[&str] = &[
    "crud-member",
    "crud-squad",
    "crud-pool",
    "crud-rename-fields",
    "crud-rename-reopened",
    "crud-rename-review",
    "crud-rename-pending",
    "crud-rename-success",
    "crud-rename-conflict",
    "crud-moving",
    "crud-moving-pool",
    "close-member-confirm",
    "close-member-refused",
    "close-member-signalled",
    "close-member-unavailable",
    "close-member-running",
    "close-member-pending",
    "close-member-pane-kept",
    "crud-leave-squad",
    "timers-delete-confirm",
    "cron-review",
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
        "close-member-confirm"
        | "close-member-refused"
        | "close-member-signalled"
        | "close-member-unavailable"
        | "close-member-running"
        | "close-member-pending"
        | "close-member-pane-kept" => {
            use crate::board::workspace::close_member::{Outcome, Step};
            app.workspace_begin(Kind::RemoveMember);
            let draft = app.workspace_form.as_mut().unwrap();
            if scene == "close-member-pending" {
                draft.stage = Stage::Pending;
            } else if scene != "close-member-confirm" {
                let (step, code, message, signalled) = match scene {
                    "close-member-signalled" => (
                        Step::Stop,
                        "PROCESS_ERROR",
                        "The agent did not exit within three seconds.",
                        Some(true),
                    ),
                    "close-member-unavailable" => (
                        Step::Stop,
                        "RESUME_UNAVAILABLE",
                        "No recorded agent session.",
                        Some(false),
                    ),
                    "close-member-running" => (
                        Step::Delete,
                        "AGENT_RUNNING",
                        "The recorded agent is still running or unproven.",
                        None,
                    ),
                    "close-member-pane-kept" => (
                        Step::Delete,
                        "API_UNAVAILABLE",
                        "Core command did not complete.",
                        None,
                    ),
                    _ => (
                        Step::Stop,
                        "RUNTIME_CONFLICT",
                        "The agent shares a runtime; quit it yourself in its pane.",
                        Some(false),
                    ),
                };
                let mut error = crate::core::SquadError::new(code, message);
                error.signalled = signalled;
                draft.stage = Stage::Result(String::new());
                draft.close.as_mut().unwrap().result = Some(Outcome::Refused {
                    step,
                    error,
                    pane_reason: (scene == "close-member-pane-kept")
                        .then(|| "The pane was opened by you.".into()),
                });
            }
        }
        "crud-leave-squad" => {
            app.workspace_begin(Kind::Remove);
        }
        // A kept Rename draft opened again: its title names the agent and Discard joins Cancel.
        // The fixture has one agent, so it reopens from the same row; the title rule holds on
        // every reopen.
        "crud-rename-reopened" => {
            app.workspace_begin(Kind::RenameMember);
            app.input.as_mut().unwrap().text = "worker-tw".into();
            key(&mut app, KeyCode::Esc);
            app.workspace_begin(Kind::RenameMember);
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
        scene if scene.starts_with("crud-moving") => {
            app.shell.tab = OpsTab::Agents;
            app.workspace.expanded.insert(ROOM.into());
            app.workspace.selected = Some(OpsTarget::Member {
                room_id: Some(ROOM.into()),
                identity_id: MEMBER.into(),
            });
            key(&mut app, KeyCode::Char('m'));
            if scene == "crud-moving-pool" {
                key(&mut app, KeyCode::Down);
            }
        }
        // The board confirm over the Timers list, on its starting Cancel.
        "timers-delete-confirm" => {
            use crate::board::cronboard::{TEST_NOW, test_cron, test_view};
            let mut job = test_view("worker", "merge queue sweep", Some(TEST_NOW + 3_600_000));
            job.squad = "product".into();
            job.job.room_id = ROOM.into();
            job.job.owner_id = Some(MEMBER.into());
            app.cron.replace(Ok(test_cron(
                vec![job],
                tmt_ops::cron::ClockStatus::NoClock,
            )));
            key(&mut app, KeyCode::Char('c'));
            key(&mut app, KeyCode::Char('d'));
            assert!(app.confirm.is_some(), "d asks before deleting");
        }
        // Timer review with its preview read, on its starting Cancel.
        "cron-review" => {
            use crate::board::cronboard::{CronRequest, preview, test_cron};
            let request = CronRequest::Add {
                actor: test_cron(vec![], tmt_ops::cron::ClockStatus::NoClock)
                    .actor
                    .unwrap(),
                squad: "product".into(),
                room_id: ROOM.into(),
                owner: "worker".into(),
                message: "sweep the merge queue".into(),
                schedule: "every 30m".into(),
                zone: "UTC".into(),
            };
            let Effect::Act(Request::CronPreview(task)) = app.cron_review_request(request) else {
                panic!("a preview read")
            };
            let (task, result) = preview::run(*task, 0);
            app.cron_review_ready(task, result);
        }
        _ => panic!("unknown CRUD scene {scene}"),
    }
    app
}

#[test]
fn member_footer_uses_effective_crud_bindings_and_never_advertises_future_actions() {
    let mut app = board();
    let text = paint(&app, 150, 43).join("\n");
    for hint in ["n new", "m move", "r rename", "x close"] {
        assert!(text.contains(hint), "{hint}: {text}");
    }
    assert!(text.contains("e edit"), "{text}");
    assert!(!text.contains("S switch tool"));
    assert!(!text.contains("R resume"));
    assert!(!text.contains("d remove"));
    app.shell.tab = OpsTab::Digest;
    let digest = paint(&app, 150, 43).join("\n");
    assert!(!digest.contains("d remove"));
    assert!(!digest.contains("n new"));
    let app = configured_member(
        "[bind]\nd = \"help\"\nz = \"remove-member\"\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    let text = paint(&app, 150, 43).join("\n");
    assert!(!text.contains("z remove") && !text.contains("d remove"));
    assert!(super::help_lists(&app, "z"));
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

#[test]
fn edit_replaces_member_s_preserves_squad_s_and_opens_before_io() {
    let mut app = configured_member(
        "",
        json!({"id":MEMBER,"name":"worker","runningDriver":"codex","location":{"host":"tmux","session":"work"}}),
    );
    app.references = crate::board::inventory::Inventory::from_listed(&Ok(json!({
        "identities": [{"id": MEMBER, "name": "worker", "runningDriver": "codex",
            "location": {"host": "tmux", "session": "work"}}]
    })));
    assert!(!app.bindings().contains_key("S"));
    assert_eq!(app.bindings()["e"].text, "edit");
    let _no_commands = crate::runner::forbid_commands();
    key(&mut app, KeyCode::Char('e'));
    let text = paint(&app, 150, 43).join("\n");
    assert!(text.contains("Edit member"));
    assert!(text.contains("Loading"));
    key(&mut app, KeyCode::Esc);
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    assert_eq!(app.workspace_effective_bindings()["S"].text, "show-squad");
    app.workspace.selected = Some(OpsTarget::Pool);
    assert!(!app.bindings().values().any(|a| a.text == "switch-tool"));
}

#[test]
fn switch_tool_respects_authored_keys_and_is_absent_outside_agents() {
    let mut app = configured_member(
        "[bind]\nS = \"help\"\nz = \"switch-tool\"\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    assert_eq!(app.bindings()["S"].text, "help");
    assert_eq!(app.bindings()["z"].text, "switch-tool");
    app.shell.tab = OpsTab::Digest;
    assert!(!app.bindings().values().any(|a| a.text == "switch-tool"));
    app.shell.tab = OpsTab::Agents;
    app.focus = 1;
    assert!(!app.bindings().values().any(|a| a.text == "switch-tool"));
}

#[test]
fn close_delete_x_opens_cancel_first_confirm_instead_of_leaving_the_squad() {
    let mut app = board();
    let _no_commands = crate::runner::forbid_commands();
    assert_eq!(key(&mut app, KeyCode::Char('x')), Effect::None);
    let screen = paint(&app, 100, 36).join("\n");
    assert!(
        screen.contains("Close and delete"),
        "x must review stopping and deleting the identity, not just leaving its squad: {screen}"
    );
    assert!(screen.contains("worker"));
    assert!(screen.contains("Conversation history and notes are kept"));
    assert!(!screen.contains("Copy command"));
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(
        !app.modal_open(),
        "initial Enter chooses Cancel and never dispatches"
    );
}

#[test]
fn close_delete_explicit_confirm_dispatches_once_instead_of_copying_a_command() {
    let mut app = board();
    let _no_commands = crate::runner::forbid_commands();
    member_menu_action(&mut app, OpsUiAction::RemoveMember);
    paint(&app, 100, 36);
    assert_eq!(key(&mut app, KeyCode::Right), Effect::None);
    paint(&app, 100, 36);
    let effect = key(&mut app, KeyCode::Enter);
    assert!(
        matches!(&effect, Effect::Act(request) if !matches!(request, Request::Copy { .. })),
        "explicit Close and delete must dispatch lifecycle work, not copy-only retirement: {effect:?}"
    );
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Effect::None,
        "pending work never resubmits"
    );
}
