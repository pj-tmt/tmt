pub(in crate::board) mod captures;
mod worker;
use super::*;
use crate::board::app::workspace::tests::board;
use crate::{config::Config, core::SquadError};
use serde_json::json;
fn loaded() -> Loaded {
    Loaded{hosts:crate::launch::hosts(&json!({"version":1,"hosts":[{"host":"herdr","state":"ok","launch":null},{"host":"tmux","state":"builtin","launch":{"argv":["host","{session}","{command}"],"placeholders":["session","command"]}}]})).unwrap(),config:Config::read(std::env::temp_dir().join(format!("ops-new-member-missing-{}",crate::id::new_v4().unwrap()))).unwrap(),executable:"/tool/tmt".into()}
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
fn null_template_copies_both_lines_without_a_launch_or_creation() {
    let app = board();
    let mut draft = model::Draft::from_app(&app);
    for (index, text) in [(0, "new-worker"), (1, "codex"), (4, "herdr"), (5, "common")] {
        draft.fields[index] = Editor::from(text);
    }
    let review = draft.review(&loaded()).unwrap();
    assert!(review.outer.is_none());
    assert!(
        review
            .copy()
            .starts_with("/tool/tmt run --save new-worker codex")
    );
    if review.room.is_some() {
        assert_eq!(review.copy().lines().count(), 2);
        assert!(review.copy().contains("ops sq add"));
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
    let original = app.new_member.as_ref().unwrap().draft.fields[3].clone();
    app.new_member.as_mut().unwrap().focus = 6;
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
            .any(|(i, _)| *i == Field::Text(3))
    );
    app.new_member.as_mut().unwrap().focus = 6;
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.new_member.as_ref().unwrap().draft.fields[3], original);
}
#[test]
fn form_and_review_are_admitted_at_design_sizes_and_launch_only_once() {
    let _ui = crate::runner::forbid_commands();
    for width in [80, 150] {
        let mut app = form();
        app.new_member.as_mut().unwrap().focus = 8;
        let screen = paint(&app, width, 43);
        assert!(
            app.new_member.as_ref().unwrap().readable.get(),
            "{width}: {screen}"
        );
        assert!(screen.contains("Will run"));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.new_member.as_ref().unwrap().stage, Stage::Review);
        paint(&app, width, 43);
        assert!(app.new_member.as_ref().unwrap().readable.get());
        key(&mut app, KeyCode::Tab);
        paint(&app, width, 43);
        assert!(matches!(
            key(&mut app, KeyCode::Enter),
            Effect::NewMember(Task {
                work: Work::Launch(_),
                ..
            })
        ));
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    }
}
#[test]
fn clipped_review_and_stale_hits_cannot_start_a_launch() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    let c = app.new_member.as_mut().unwrap();
    c.review = Some(c.draft.review(c.loaded.as_ref().unwrap()).unwrap());
    c.stage = Stage::Review;
    c.focus = 1;
    paint(&app, 80, 8);
    assert!(!app.new_member.as_ref().unwrap().readable.get());
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(matches!(
        app.new_member.as_ref().unwrap().stage,
        Stage::Review
    ));
}
#[test]
fn template_editor_returns_to_the_exact_member_draft_without_saving_on_escape() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    let draft = app.new_member.as_ref().unwrap().draft.fields.clone();
    app.new_member.as_mut().unwrap().focus = 4;
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
    assert_eq!(review.command[3], "my worker's $token; hi");
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
    app.references = Inventory::from_listed(&[], &Ok(rows));
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
fn copy_uses_the_exact_reviewed_host_argv_with_no_reconstruction() {
    let app = form();
    let c = app.new_member.as_ref().unwrap();
    let review = c.draft.review(c.loaded.as_ref().unwrap()).unwrap();
    assert_eq!(
        review.copy().lines().next().unwrap(),
        crate::launch::line(review.outer.as_ref().unwrap())
    );
}

#[test]
fn host_selector_cycles_without_typing_over_its_published_value() {
    let _ui = crate::runner::forbid_commands();
    let mut app = form();
    app.new_member.as_mut().unwrap().focus = 4;
    key(&mut app, KeyCode::Char('h'));
    assert_eq!(
        app.new_member.as_ref().unwrap().draft.fields[4].as_str(),
        "tmux"
    );
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(
        app.new_member.as_ref().unwrap().draft.fields[4].as_str(),
        "herdr"
    );
    key(&mut app, KeyCode::Char('e'));
    assert_eq!(app.new_member.as_ref().unwrap().stage, Stage::Template);
}
