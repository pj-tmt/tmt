pub(in crate::board) mod captures;
mod worker;
use super::*;
use crate::board::app::workspace::tests::board;
use crate::{config::Config, core::SquadError};
use serde_json::json;
fn loaded() -> Loaded {
    Loaded{caller_host:Default::default(),hosts:crate::launch::hosts(&json!({"version":1,"hosts":[{"host":"herdr","state":"ok","launch":null},{"host":"tmux","state":"builtin","launch":{"argv":["host","{session}","{command}"],"placeholders":["session","command"]}},{"host":"pty","state":"builtin","launch":{"argv":["pty-host","{session}","{command}"],"placeholders":["session","command"]}}]})).unwrap(),config:Config::read(std::env::temp_dir().join(format!("ops-new-member-missing-{}",crate::id::new_v4().unwrap()))).unwrap(),executable:"/tool/tmt".into(),display_executable:"tmt".into(),
    choices:[("codex".to_owned(),crate::board::host_launch::LaunchChoices{models:vec!["gpt-6.1-sol".into(),"gpt-6.1-terra".into()],efforts:vec!["low".into(),"medium".into(),"high".into()]})].into()}
}
#[test]
fn pool_defaults_to_first_eligible_published_host_and_common_not_running() {
    let mut app = board();
    app.workspace.selected = Some(OpsTarget::Pool);
    let Effect::NewMember(task) = app.open_new_member() else {
        panic!()
    };
    let mut observed = loaded();
    observed.hosts.insert(
        0,
        crate::launch::Host {
            name: "missing".into(),
            state: "missing".into(),
            argv: None,
        },
    );
    app.finished_new_member(Completed {
        task,
        result: Ok(Outcome::Loaded(Box::new(observed))),
    });
    let c = app.new_member.as_ref().unwrap();
    assert_eq!(c.draft.fields[4].as_str(), "tmux");
    assert_eq!(c.draft.fields[5].as_str(), "common");
    assert!(c.draft.saved);
}
#[test]
fn the_host_starts_on_the_environment_ops_runs_in() {
    use crate::board::host_launch::CallerHost;
    let default = |caller_host: CallerHost, herdr_launches: bool| {
        let mut app = board();
        let Effect::NewMember(task) = app.open_new_member() else {
            panic!()
        };
        let mut observed = loaded();
        observed.caller_host = caller_host;
        if herdr_launches {
            observed.hosts[0].argv = Some(vec!["herdr-open".into(), "{command}".into()]);
        }
        app.finished_new_member(Completed {
            task,
            result: Ok(Outcome::Loaded(Box::new(observed))),
        });
        app.new_member
            .as_ref()
            .unwrap()
            .draft
            .text(model::Slot::Host)
            .to_owned()
    };
    let named = |host: &str| CallerHost::Named(host.into());
    assert_eq!(
        default(CallerHost::Plain, true),
        "pty",
        "no tmux: the built-in terminal"
    );
    assert_eq!(default(named("tmux"), true), "tmux");
    assert_eq!(
        default(named("pty"), true),
        "pty",
        "inside a built-in member"
    );
    assert_eq!(default(named("herdr"), true), "herdr");
    // A host Ops runs in but cannot launch on (no template) falls back.
    assert_eq!(default(named("herdr"), false), "tmux");
    // An older Core does not say: unchanged.
    assert_eq!(
        default(CallerHost::Unknown, true),
        "herdr",
        "first eligible"
    );
    // Only null is a plain terminal; ambiguous locators omit the field, and a value this Ops
    // does not know keeps the default too.
    let read = |document| CallerHost::of(&document);
    assert_eq!(read(json!({"callerHost": null})), CallerHost::Plain);
    assert_eq!(
        read(json!({})),
        CallerHost::Unknown,
        "omitted when ambiguous"
    );
    assert_eq!(read(json!({"callerHost": 1})), CallerHost::Unknown);
    assert_eq!(read(json!({"callerHost": "herdr"})), named("herdr"));
}
#[test]
fn null_template_copies_both_lines_without_a_launch_or_creation() {
    let app = board();
    let mut draft = model::Draft::from_app(&app);
    for (index, text) in [(0, "new-worker"), (1, "codex"), (4, "herdr"), (5, "common")] {
        draft.fields[index] = Editor::from(text);
    }
    let review = draft.review(&loaded()).unwrap();
    assert!(review.outer.is_none());
    // Saved members launch temporary and are kept once started (#5536).
    let copied = review.copy();
    let lines: Vec<_> = copied.lines().collect();
    assert_eq!(lines[0], "tmt run new-worker codex");
    assert_eq!(lines[1], "tmt identity create new-worker");
    if review.room.is_some() {
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("ops sq add"));
    }
}
#[test]
fn a_late_read_cannot_replace_the_new_form() {
    let mut app = board();
    let Effect::NewMember(old) = app.open_new_member() else {
        panic!()
    };
    app.new_member = None;
    let Effect::NewMember(_) = app.open_new_member() else {
        panic!()
    };
    app.finished_new_member(Completed {
        task: old,
        result: Err(SquadError::new("TEST", "old read")),
    });
    assert!(app.new_member.as_ref().unwrap().notice.is_none());
}
fn key(app: &mut App, code: ratatui::crossterm::event::KeyCode) -> Effect {
    app.overlay_event(&Event::Key(ratatui::crossterm::event::KeyEvent::new(
        code,
        KeyModifiers::NONE,
    )))
    .unwrap()
}
fn paint(app: &App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
fn form() -> App {
    let mut app = board();
    let Effect::NewMember(task) = app.open_new_member() else {
        panic!()
    };
    app.finished_new_member(Completed {
        task,
        result: Ok(Outcome::Loaded(Box::new(loaded()))),
    });
    let c = app.new_member.as_mut().unwrap();
    for (index, text) in [(0, "new-worker"), (1, "codex"), (4, "tmux"), (5, "common")] {
        c.draft.fields[index] = Editor::from(text);
    }
    app
}
#[test]
fn temporary_disables_squad_preserves_saved_choice_and_null_copy_has_no_add() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    let original = app.new_member.as_ref().unwrap().draft.fields[6].clone();
    app.new_member.as_mut().unwrap().at = Focus::Slot(model::Slot::Keep);
    key(&mut app, KeyCode::Char(' '));
    let c = app.new_member.as_mut().unwrap();
    assert!(!c.draft.saved);
    c.draft.fields[4] = Editor::from("herdr");
    let review = c.draft.review(c.loaded.as_ref().unwrap()).unwrap();
    assert!(review.room.is_none());
    assert_eq!(review.copy().lines().count(), 1);
    assert!(!review.copy().contains("--save"));
    paint(&app, 150, 43);
    assert!(
        !app.new_member
            .as_ref()
            .unwrap()
            .fields
            .borrow()
            .iter()
            .any(|(i, _)| *i == Field::Slot(model::Slot::Squad))
    );
    app.new_member.as_mut().unwrap().at = Focus::Slot(model::Slot::Keep);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.new_member.as_ref().unwrap().draft.fields[6], original);
}
#[test]
fn the_form_is_admitted_at_design_sizes_and_create_launches_only_once() {
    let _ui = crate::runner::forbid_commands();
    for width in [80, 150] {
        let mut app = form();
        app.new_member.as_mut().unwrap().at = Focus::Create;
        let screen = paint(&app, width, 43);
        assert!(
            app.new_member.as_ref().unwrap().readable.get(),
            "{width}: {screen}"
        );
        assert!(screen.contains("Runs"));
        // No Review step: Create starts the launch.
        let effect = key(&mut app, KeyCode::Enter);
        assert!(
            matches!(
                effect,
                Effect::NewMember(Task {
                    work: Work::Launch(_),
                    ..
                })
            ),
            "{effect:?} {:?}",
            app.new_member.as_ref().map(|c| (&c.notice, &c.stage, c.at))
        );
        assert_eq!(app.new_member.as_ref().unwrap().stage, Stage::Launching);
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    }
}
#[test]
fn a_clipped_form_and_stale_hits_cannot_start_a_launch() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    app.new_member.as_mut().unwrap().at = Focus::Create;
    paint(&app, 80, 8);
    assert!(!app.new_member.as_ref().unwrap().readable.get());
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.new_member.as_ref().unwrap().stage, Stage::Form);
}
#[test]
fn arrows_reach_every_shown_field_and_both_buttons() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    let stops = |app: &mut App| {
        let mut seen = vec![app.new_member.as_ref().unwrap().at];
        loop {
            key(app, KeyCode::Down);
            let at = app.new_member.as_ref().unwrap().at;
            if at == seen[0] {
                return seen;
            }
            seen.push(at);
        }
    };
    use model::Slot::*;
    let slots = |slots: &[model::Slot]| {
        slots
            .iter()
            .map(|s| Focus::Slot(*s))
            .chain([Focus::Cancel, Focus::Create])
            .collect::<Vec<_>>()
    };
    // tmux shows its Session; codex has efforts.
    assert_eq!(
        stops(&mut app),
        slots(&[Name, Tool, Model, Effort, Host, Session, Squad, Keep])
    );
    key(&mut app, KeyCode::Up);
    assert_eq!(app.new_member.as_ref().unwrap().at, Focus::Create);
    // The built-in host has no session; a tool without efforts hides Effort.
    let c = app.new_member.as_mut().unwrap();
    c.draft.set(Host, model::BUILT_IN_HOST);
    c.draft.set(Tool, "claude");
    c.at = Focus::Slot(Name);
    assert_eq!(
        stops(&mut app),
        slots(&[Name, Tool, Model, Host, Squad, Keep])
    );
}
#[test]
fn a_list_filters_takes_free_form_text_and_escape_closes_it_first() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    app.new_member.as_mut().unwrap().at = Focus::Slot(model::Slot::Model);
    key(&mut app, KeyCode::Enter);
    let labels = |app: &App| {
        app.new_member
            .as_ref()
            .unwrap()
            .rows()
            .into_iter()
            .map(|c| c.label)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(&app),
        [
            "(provider default)",
            "gpt-6.1-sol",
            "gpt-6.1-terra",
            "Type a model id…"
        ]
    );
    for c in "terra".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    assert_eq!(labels(&app), ["gpt-6.1-terra", "Use “terra”"]);
    key(&mut app, KeyCode::Enter);
    let c = app.new_member.as_ref().unwrap();
    assert!(c.list.is_none());
    assert_eq!(c.draft.text(model::Slot::Model), "gpt-6.1-terra");
    // A model the driver does not list is typed into the free-form row.
    key(&mut app, KeyCode::Enter);
    for c in "my-model".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.new_member
            .as_ref()
            .unwrap()
            .draft
            .text(model::Slot::Model),
        "my-model"
    );
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Esc);
    assert!(app.new_member.as_ref().unwrap().list.is_none());
    key(&mut app, KeyCode::Esc);
    assert!(app.new_member.is_none());
}
#[test]
fn the_command_passes_model_and_effort_before_the_name_and_a_new_squad_is_created() {
    let mut app = form();
    let c = app.new_member.as_mut().unwrap();
    c.draft.set(model::Slot::Effort, "high");
    c.draft.set(model::Slot::Model, "gpt-6.1-sol");
    c.draft.set(model::Slot::Squad, "fresh");
    let review = c.draft.review(c.loaded.as_ref().unwrap()).unwrap();
    assert_eq!(
        review.command[1..],
        [
            "run",
            "--model",
            "gpt-6.1-sol",
            "--effort",
            "high",
            "new-worker",
            "codex"
        ]
    );
    assert_eq!(review.new_squad.as_deref(), Some("fresh"));
    let copied = review.copy();
    assert!(copied.contains("tmt ops sq init fresh"), "{copied}");
    assert!(copied.contains("tmt ops sq add --squad fresh -- new-worker"));
    // Another tool forgets the model and effort it cannot use.
    c.draft.set(model::Slot::Tool, "claude");
    assert_eq!(c.draft.text(model::Slot::Model), "");
    assert_eq!(c.draft.text(model::Slot::Effort), "");
    // A session that would read as an option is refused too.
    c.draft.set(model::Slot::Session, "-L");
    let refused = c.draft.review(c.loaded.as_ref().unwrap()).unwrap_err();
    assert!(refused.contains("session"), "{refused}");
    c.draft.set(model::Slot::Session, "common");
    // A model that would read as an option is refused.
    c.draft.set(model::Slot::Model, "--resume");
    assert!(c.draft.review(c.loaded.as_ref().unwrap()).is_err());
}
#[test]
fn a_clean_built_in_start_closes_the_form_onto_the_member_terminal() {
    let mut app = form();
    let c = app.new_member.as_mut().unwrap();
    c.draft.set(model::Slot::Host, model::BUILT_IN_HOST);
    let review = c.draft.review(c.loaded.as_ref().unwrap()).unwrap();
    let task = c.task(Work::Launch(review));
    app.right_tab = crate::board::shell::RightTab::Measure;
    let effect = app.finished_new_member(Completed {
        task,
        result: Ok(Outcome::Created {
            id: "33333333-3333-4333-8333-333333333333".into(),
            room: None,
            message: "Created new-worker in Common pool.".into(),
        }),
    });
    assert_eq!(effect, Effect::Refresh);
    assert!(app.new_member.is_none());
    assert_eq!(app.right_tab, crate::board::shell::RightTab::Talk);
    assert!(app.terminal.shown());
    // Its stream types once it opens; the terminal tests drive that.
    assert_eq!(
        app.terminal.typing_wish(),
        Some("33333333-3333-4333-8333-333333333333")
    );
    // A tmux start, or one with a partial failure, stays on the result.
    let mut app = form();
    let c = app.new_member.as_ref().unwrap();
    let review = c.draft.review(c.loaded.as_ref().unwrap()).unwrap();
    let task = c.task(Work::Launch(review));
    app.finished_new_member(Completed {
        task,
        result: Ok(Outcome::Created {
            id: "33333333-3333-4333-8333-333333333333".into(),
            room: None,
            message: "Created new-worker in Common pool.".into(),
        }),
    });
    assert!(matches!(
        app.new_member.as_ref().unwrap().stage,
        Stage::Result(_)
    ));
}
#[test]
fn template_editor_returns_to_the_exact_member_draft_without_saving_on_escape() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    let draft = app.new_member.as_ref().unwrap().draft.fields.clone();
    app.new_member.as_mut().unwrap().at = Focus::Slot(model::Slot::Host);
    key(&mut app, KeyCode::Char('e'));
    assert_eq!(app.new_member.as_ref().unwrap().stage, Stage::Template);
    app.new_member_event(&Event::Paste(" invalid".into()));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.new_member.as_ref().unwrap().draft.fields, draft);
}

#[test]
fn quoted_name_and_tool_path_remain_literal_inner_words() {
    let mut app = form();
    let c = app.new_member.as_mut().unwrap();
    c.draft.fields[0] = Editor::from("my worker's $token; hi");
    c.draft.fields[1] = Editor::from("/tools/my tool");
    let review = c.draft.review(c.loaded.as_ref().unwrap()).unwrap();
    assert_eq!(review.command[2], "my worker's $token; hi");
    assert!(crate::launch::line(&review.command).contains("'/tools/my tool'"));
}
#[test]
fn defaults_deduplicate_members_and_choose_observed_model_only_within_the_tool() {
    use crate::board::{
        app::workspace::tests::{MEMBER, ROOM},
        inventory::Inventory,
    };
    let mut app = board();
    let rows = json!({"identities":[
        {"id":MEMBER,"name":"worker","runningDriver":"codex","resume":{"model":"old-start","observedModel":{"model":"beta","observedAtMs":100}}},
        {"id":"other","name":"other","runningDriver":"codex","resume":{"observedModel":{"model":"alpha","observedAtMs":100}}},
        {"id":"claude","name":"claude","runningDriver":"claude","resume":{"observedModel":{"model":"zeta","observedAtMs":100}}}
    ]});
    app.references = Inventory::from_listed(&Ok(rows));
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot
        .roster
        .squads
        .iter_mut()
        .find(|s| s.room_id == ROOM)
        .unwrap()
        .members = vec![
        MEMBER.into(),
        "other".into(),
        "claude".into(),
        "claude".into(),
        "claude".into(),
    ];
    let draft = model::Draft::from_app(&app);
    assert_eq!(draft.fields[1].as_str(), "codex");
    assert_eq!(draft.fields[2].as_str(), "beta");
    assert_eq!(draft.models["codex"], vec!["alpha", "beta"]);
    assert!(!draft.models["codex"].contains(&"old-start".into()));
}

#[test]
fn copy_uses_the_reviewed_human_host_command() {
    let app = form();
    let c = app.new_member.as_ref().unwrap();
    let review = c.draft.review(c.loaded.as_ref().unwrap()).unwrap();
    assert_eq!(
        review.copy().lines().next().unwrap(),
        crate::launch::line(review.display.outer.as_ref().unwrap())
    );
}

#[test]
fn host_selector_cycles_without_typing_over_its_published_value() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    app.new_member.as_mut().unwrap().at = Focus::Slot(model::Slot::Host);
    key(&mut app, KeyCode::Char('h'));
    assert_eq!(
        app.new_member.as_ref().unwrap().draft.fields[4].as_str(),
        "tmux"
    );
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(
        app.new_member.as_ref().unwrap().draft.fields[4].as_str(),
        "pty"
    );
    key(&mut app, KeyCode::Char('e'));
    assert_eq!(app.new_member.as_ref().unwrap().stage, Stage::Template);
}

#[test]
fn reviewed_commands_display_path_alias_without_changing_execution() {
    let app = form();
    let c = app.new_member.as_ref().unwrap();
    let review = c.draft.review(c.loaded.as_ref().unwrap()).unwrap();
    assert_eq!(review.command[0], "/tool/tmt");
    assert!(
        review
            .outer
            .as_ref()
            .unwrap()
            .last()
            .unwrap()
            .starts_with("/tool/tmt run")
    );
    assert!(review.copy().contains("tmt run"));
    assert!(!review.copy().contains("/tool/tmt"));
    let mut different = loaded();
    different.display_executable = different.executable.clone();
    assert!(
        c.draft
            .review(&different)
            .unwrap()
            .copy()
            .contains("/tool/tmt")
    );
}

#[test]
fn dialogs_use_compact_content_and_the_approved_copy() {
    let _ui = crate::runner::forbid_commands();
    for width in [80, 150] {
        for scene in captures::SCENES {
            let app = captures::case(scene, crate::look::Look::default());
            let text = paint(&app, width, 43);
            assert!(!text.contains("Create member"), "{scene}");
            let c = app.new_member.as_ref().unwrap();
            let area = app.input_band.get().unwrap();
            match *scene {
                "new-member-loading" | "new-member-load-failed" => {
                    assert!(!text.contains("Review"), "{scene}");
                    assert!(!text.contains("Tab next"), "{scene}");
                    assert!(c.controls.get()[1].is_none());
                    assert!(area.height <= 6, "{scene}: {area:?}");
                    if *scene == "new-member-load-failed" {
                        assert!(
                            text.contains("Host templates unavailable. Close and reopen to retry.")
                        );
                        assert!(!text.contains("No launch attempted"));
                    }
                }
                "new-member-copy-only" | "new-member-copy-temporary" => {
                    assert!(text.contains("Copy only · this host has no launch template"));
                    assert!(text.contains("Copy commands"));
                    assert!(!text.contains("Runs"));
                }
                "new-member-settings" => {
                    // People read the host's name; `pty` stays in commands only.
                    assert!(
                        text.contains("Built-in terminal (no tmux) · builtin"),
                        "{text}"
                    );
                    assert!(!text.contains("pty · builtin"));
                }
                "new-member-form" => {
                    assert!(text.contains("Runs · e edit template"));
                    assert!(!text.contains("/tool/tmt"));
                }
                "new-member-built-in" => {
                    assert!(text.contains("Built-in terminal (no tmux)"));
                    assert!(!text.contains("Session"));
                    assert!(text.contains("--effort high new-worker codex"), "{text}");
                }
                "new-member-list-filtered" => {
                    assert!(text.contains("Model · type to filter"));
                    assert!(text.contains("Use “terra”"));
                    assert!(!text.contains("gpt-6.1-sol"));
                }
                "new-member-pending" => {
                    assert!(text.contains("Starting new-worker in tmux / common…"));
                    assert!(!text.contains("UUID"));
                    assert!(!text.contains("retry"));
                    assert!(area.height <= 6, "{area:?}");
                }
                _ => {}
            }
        }
    }
}

#[test]
fn the_new_member_form_follows_the_close_rule() {
    crate::board::home::tests::dialog_dismiss::assert_close_rule(
        &mut form(),
        &mut form(),
        crate::board::app::Overlay::NewMember,
    );
}
#[test]
fn esc_hides_a_running_launch_and_the_status_line_follows_it_to_its_result() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    app.new_member.as_mut().unwrap().at = Focus::Create;
    paint(&app, 150, 43);
    let Effect::NewMember(task) = key(&mut app, KeyCode::Enter) else {
        panic!("Create starts the launch")
    };
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay(), None, "Esc hides the running launch");
    let c = app.new_member.as_ref().expect("the launch keeps going");
    assert_eq!(c.stage, Stage::Launching);
    let running = app.notice.clone().unwrap_or_default();
    assert!(
        running.starts_with("◌ Launching · new-worker.")
            && running.ends_with("n reopens this new member or its result."),
        "{running}"
    );
    app.finished_new_member(Completed {
        task,
        result: Ok(Outcome::Created {
            id: "33333333-3333-4333-8333-333333333333".into(),
            room: None,
            message: "Created new-worker in Common pool.\nStarted in tmux.".into(),
        }),
    });
    let finished = app.notice.clone().unwrap_or_default();
    assert_eq!(
        finished,
        "Created new-worker in Common pool. · n reopens this new member or its result."
    );
    assert_eq!(
        app.overlay(),
        None,
        "the result waits behind the closed form"
    );
    assert_eq!(
        app.open_new_member(),
        Effect::None,
        "reopening starts nothing"
    );
    assert!(matches!(
        app.new_member.as_ref().unwrap().stage,
        Stage::Result(_)
    ));
    assert_eq!(app.overlay(), Some(crate::board::app::Overlay::NewMember));
}
