pub(in crate::board) mod captures;
mod worker;
use super::*;
use crate::board::{
    app::workspace::tests::{MEMBER, board},
    inventory::Inventory,
};
use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyEvent};
use serde_json::json;
fn row() -> serde_json::Value {
    json!({"id":MEMBER,"name":"worker","driverSupport":"full","runningDriver":"codex","location":{"host":"tmux","session":"work","window":"old"},"resume":{"driver":"codex","session":"old-session","staleAtMs":null}})
}
fn app() -> App {
    let mut app = board();
    app.references = Inventory::from_listed(&[], &Ok(json!({"identities":[row()]})));
    app
}
fn loaded() -> Loaded {
    Loaded{hosts:crate::launch::hosts(&json!({"version":1,"hosts":[{"host":"tmux","state":"builtin","launch":{"argv":["host","{session}","{command}"],"placeholders":["session","command"]}},{"host":"herdr","state":"ok","launch":null}]})).unwrap(),
        config:crate::config::Config::read(std::env::temp_dir().join(format!("ops-switch-missing-{}",crate::id::new_v4().unwrap()))).unwrap(),executable:"/tool/tmt".into(),display_executable:"tmt".into()}
}
fn form() -> App {
    let mut app = app();
    let Effect::SwitchTool(task) = app.open_switch_tool() else {
        panic!()
    };
    app.finished_switch_tool(Completed {
        task,
        result: Ok(Outcome::Loaded(Box::new(loaded()))),
    });
    app.switch_tool.as_mut().unwrap().fields[0] = Editor::from("claude");
    app
}
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.switch_tool_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap()
}
fn paint(app: &App, width: u16, height: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(width, height)).unwrap();
    t.draw(|f| super::super::view::render(f, app)).unwrap();
    t.backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
fn review(app: &mut App) {
    app.switch_tool.as_mut().unwrap().focus = 3;
    paint(app, 150, 43);
    key(app, KeyCode::Enter);
    assert_eq!(app.switch_tool.as_ref().unwrap().stage, Stage::Review);
    paint(app, 150, 43);
}
#[test]
fn review_cancel_default_copies_literal_argv_and_never_executes_on_ui_thread() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    app.switch_tool.as_mut().unwrap().fields[1] = Editor::from("a'b $c; d");
    review(&mut app);
    assert!(paint(&app, 80, 43).contains("Switch AI tool"));
    let Effect::Act(Request::Copy { text, .. }) = key(&mut app, KeyCode::Char('c')) else {
        panic!()
    };
    assert!(text.starts_with("host work "));
    assert!(text.contains("tmt resume --driver=claude"));
    assert!(text.contains("$c; d"));
    assert!(text.contains(MEMBER));
    assert!(!text.contains("/tool/tmt"));
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.switch_tool.as_ref().unwrap().stage, Stage::Form);
    assert!(app.foreground.is_none());
}
#[test]
fn changing_tool_never_carries_another_tools_model_and_empty_omits_flag() {
    let mut app = form();
    let c = app.switch_tool.as_mut().unwrap();
    c.fields[1] = Editor::from("old-model");
    c.focus = 0;
    key(&mut app, KeyCode::Char('x'));
    assert!(app.switch_tool.as_ref().unwrap().fields[1].is_empty());
    let source = Source::from_app(&app).unwrap();
    let review = Review::new(&source, "claude", "", &loaded()).unwrap();
    assert!(!review.command.iter().any(|v| v.starts_with("--model")));
}
#[test]
fn live_activity_does_not_invalidate_but_source_session_driver_and_selection_do() {
    let mut app = form();
    review(&mut app);
    app.references.agents.get_mut(MEMBER).unwrap().parameters["resume"]["usage"] =
        json!({"tokens":900});
    app.switch_tool_changed();
    assert_eq!(app.switch_tool.as_ref().unwrap().stage, Stage::Review);
    for path in ["driver", "session", "staleAtMs"] {
        let mut app = form();
        review(&mut app);
        app.references.agents.get_mut(MEMBER).unwrap().parameters["resume"][path] =
            json!("changed");
        key(&mut app, KeyCode::Tab);
        assert!(matches!(
            app.switch_tool.as_ref().unwrap().stage,
            Stage::Result(_)
        ));
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    }
}
#[test]
fn pending_cannot_replay_and_late_completion_cannot_replace_another_owner() {
    let mut app = form();
    review(&mut app);
    key(&mut app, KeyCode::Tab);
    paint(&app, 150, 43);
    let Effect::SwitchTool(task) = key(&mut app, KeyCode::Enter) else {
        panic!()
    };
    assert!(matches!(task.work, Work::Launch(_)));
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    assert_eq!(app.switch_tool.as_ref().unwrap().stage, Stage::Pending);
    app.switch_tool = None;
    let Effect::SwitchTool(_) = app.open_switch_tool() else {
        panic!()
    };
    app.finished_switch_tool(Completed {
        task,
        result: Ok(Outcome::Switched("old result".into())),
    });
    assert_eq!(app.switch_tool.as_ref().unwrap().stage, Stage::Form);
    assert!(app.switch_tool.as_ref().unwrap().loaded.is_none());
}
#[test]
fn null_template_is_copy_only_and_missing_destination_is_never_substituted() {
    let mut app = app();
    app.references.agents.get_mut(MEMBER).unwrap().parameters["location"]["host"] = json!("herdr");
    let source = Source::from_app(&app).unwrap();
    let review = Review::new(&source, "claude", "", &loaded()).unwrap();
    assert!(review.outer.is_none());
    assert!(review.copy().starts_with("tmt resume"));
    app.references.agents.get_mut(MEMBER).unwrap().parameters["location"]["session"] = json!(null);
    assert!(Review::new(&Source::from_app(&app).unwrap(), "claude", "", &loaded()).is_err());
}
#[test]
fn completion_needs_same_uuid_destination_and_running_driver_not_remembered_driver() {
    let review = Review::new(&Source::from_app(&app()).unwrap(), "claude", "", &loaded()).unwrap();
    let mut observed = row();
    observed["resume"]["driver"] = json!("claude");
    assert!(!review.observed(&observed));
    observed["runningDriver"] = json!("claude");
    assert!(review.observed(&observed));
    observed["id"] = json!("other");
    assert!(!review.observed(&observed));
    observed["id"] = json!(MEMBER);
    observed["location"]["session"] = json!("elsewhere");
    assert!(!review.observed(&observed));
}

#[test]
fn a_changed_selection_fences_confirmation_and_resize_drops_old_hits() {
    let mut app = form();
    review(&mut app);
    app.invalidate_frame_geometry();
    app.switch_tool.as_mut().unwrap().focus = 1;
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    paint(&app, 150, 43);
    app.workspace.selected = Some(crate::board::workspace::OpsTarget::Pool);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(matches!(
        app.switch_tool.as_ref().unwrap().stage,
        Stage::Result(ref text) if text.starts_with("✗ Changed")
    ));
    paint(&app, 150, 43);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(
        app.switch_tool.is_none(),
        "Enter closes the refusal, never launches"
    );
}

#[test]
fn captured_copy_only_review_has_a_current_source_and_copies_without_launching() {
    let mut app = captures::case("switch-tool-copy-only", crate::look::Look::default());
    assert!(
        app.switch_tool
            .as_ref()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .still_selected(&app)
    );
    paint(&app, 150, 43);
    key(&mut app, KeyCode::Tab);
    paint(&app, 150, 43);
    let Effect::Act(Request::Copy { text, .. }) = key(&mut app, KeyCode::Enter) else {
        panic!("copy, never host launch")
    };
    assert!(text.starts_with("tmt resume --driver=claude -- "));
    assert!(matches!(
        app.switch_tool.as_ref().unwrap().stage,
        Stage::Result(_)
    ));
}

#[test]
fn modal_revision_pending_hides_reopens_and_keeps_one_outcome() {
    let mut app = form();
    review(&mut app);
    key(&mut app, KeyCode::Tab);
    paint(&app, 150, 43);
    let Effect::SwitchTool(task) = key(&mut app, KeyCode::Enter) else {
        panic!()
    };
    key(&mut app, KeyCode::Esc);
    assert!(
        app.overlay().is_none(),
        "Esc hides without cancelling the accepted switch"
    );
    app.references.agents.get_mut(MEMBER).unwrap().parameters["runningDriver"] = json!("claude");
    assert_eq!(
        app.open_switch_tool(),
        Effect::None,
        "reopen does not load or dispatch again"
    );
    assert_eq!(app.switch_tool.as_ref().unwrap().owner, task.owner);
    assert_eq!(app.switch_tool.as_ref().unwrap().stage, Stage::Pending);
    key(&mut app, KeyCode::Esc);
    app.finished_switch_tool(Completed {
        task,
        result: Ok(Outcome::Switched(
            "worker is running claude in tmux / work".into(),
        )),
    });
    assert!(
        app.overlay().is_none(),
        "background completion does not steal focus"
    );
    assert_eq!(app.open_switch_tool(), Effect::None);
    let text = paint(&app, 80, 43);
    assert!(text.contains("✓ Switched · worker is running claude"));
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(app.switch_tool.is_none());
}

#[test]
fn modal_revision_current_tool_cannot_review_and_copy_only_has_no_repeated_hint() {
    let mut app = form();
    app.switch_tool.as_mut().unwrap().fields[0] = Editor::from("codex");
    let text = paint(&app, 150, 43);
    assert!(text.contains("Switch worker from codex"));
    assert!(!text.contains("Switch worker from codex to"));
    assert!(text.contains("codex · current"));
    assert!(app.switch_tool.as_ref().unwrap().controls.get()[1].is_none());
    app.switch_tool.as_mut().unwrap().focus = 3;
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.switch_tool.as_ref().unwrap().stage, Stage::Form);
    let copy = captures::case("switch-tool-copy-only", crate::look::Look::default());
    let text = paint(&copy, 150, 43);
    assert!(text.contains("Copy command"));
    assert!(!text.contains("c copy command"));
}

#[test]
fn modal_revision_hide_hint_uses_the_effective_switch_binding() {
    let configured = crate::board::app::workspace::tests::configured_member(
        "[bind]\nS=\"help\"\nz=\"switch-tool\"\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    assert_eq!(
        configured.switch_tool_reopen_hint(),
        "z reopens this switch or its result."
    );
}
