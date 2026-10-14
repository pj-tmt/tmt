use super::*;
use crate::board::app::tests::crew;
use serde_json::json;

fn app() -> App {
    let mut app = crew(Default::default());
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.view.as_mut().unwrap().me_id = Some("ben-id".into());
    app
}
fn open(app: &mut App, name: &str) {
    app.ask(
        String::new(),
        Compose::Talk { to: name.into() },
        "product".into(),
    );
}
fn close(app: &mut App) {
    app.remember_draft();
    app.input = None;
}

#[test]
fn switching_recipients_retains_independent_session_drafts() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "first\nmessage".into();
    open(&mut app, "docs");
    assert!(app.input.as_ref().unwrap().text.is_empty());
    app.input.as_mut().unwrap().text = "second".into();
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text, "first\nmessage");
    open(&mut app, "docs");
    assert_eq!(app.input.as_ref().unwrap().text, "second");
}

#[test]
fn refresh_cannot_move_a_draft_to_a_new_sender_or_reused_name() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "private draft".into();
    app.view.as_mut().unwrap().me = Some("Other".into());
    app.view.as_mut().unwrap().me_id = Some("other-id".into());
    close(&mut app);
    open(&mut app, "auth-fix");
    assert!(app.input.as_ref().unwrap().text.is_empty());
    close(&mut app);
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.view.as_mut().unwrap().me_id = Some("ben-id".into());
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["id"] = json!("replacement");
    open(&mut app, "auth-fix");
    assert!(app.input.as_ref().unwrap().text.is_empty());
    close(&mut app);
    let row = &mut app.view.as_mut().unwrap().document["sections"][0]["rows"][0];
    row["id"] = json!("auth-fix");
    row["name"] = json!("renamed");
    open(&mut app, "renamed");
    assert_eq!(app.input.as_ref().unwrap().text, "private draft");
}

#[test]
fn failure_retains_and_success_clears_only_the_submitted_revision() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "submitted".into();
    app.remember_draft();
    let input = app.input.take().unwrap();
    app.sending_draft(&input);
    app.finish_draft(false);
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text, "submitted");
    app.sending_draft(&input);
    app.input.as_mut().unwrap().text = "new revision".into();
    close(&mut app);
    app.finish_draft(true);
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text, "new revision");
    app.remember_draft();
    let input = app.input.take().unwrap();
    app.sending_draft(&input);
    app.finish_draft(true);
    open(&mut app, "auth-fix");
    assert!(app.input.as_ref().unwrap().text.is_empty());
}

#[test]
fn bracketed_paste_is_exact_folded_and_never_submits() {
    use crate::board::app::{Effect, Request};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = app();
    open(&mut app, "auth-fix");
    let text = format!("  \r\n{}\n\x1b[31m\n  ", "x".repeat(100 * 1024));
    assert_eq!(app.paste(&text), Effect::None);
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), text);
    assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
    assert!(app.input.as_ref().unwrap().text.layout(80).lines.len() < 3);
    close(&mut app);
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
    assert!(
        matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(Request::Talk { text: sent, to, .. }) if sent == text && to == "auth-fix")
    );
}

#[test]
fn closed_nonempty_drafts_hold_automatic_reload_until_explicitly_cleared() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "unfinished".into();
    close(&mut app);
    assert!(app.reload_blocked());
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text.clear();
    close(&mut app);
    assert!(!app.composer.has_drafts());
}

#[test]
fn unrelated_action_feedback_does_not_clear_a_pending_message() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "unfinished".into();
    app.remember_draft();
    let input = app.input.take().unwrap();
    app.sending_draft(&input);
    app.finished(Ok("Configuration saved".into()));
    assert!(app.composer.pending.is_some());
    assert!(app.composer.has_drafts());
    app.finish_draft(false);
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text, "unfinished");
}
