use super::*;
use crate::board::{
    ActionOutcome, BoardEvent,
    app::{Compose, Effect},
    lane,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::json;

struct Fake {
    core: crate::core::Core,
    root: std::path::PathBuf,
    calls: std::path::PathBuf,
}
impl Fake {
    fn new(document: serde_json::Value, failure: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "ops-chat-feedback-{}",
            crate::id::new_v4().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("tmt");
        let calls = root.join("calls");
        let document = document.to_string().replace('\'', "'\"'\"'");
        crate::test_support::write_ready_executable(
            &executable,
            &format!(
                "#!/bin/sh\nprintf '%s\\0' \"$@\" >> '{}'\nprintf '%s\\n' '{}'\nexit {}\n",
                calls.display(),
                document,
                usize::from(failure)
            ),
        );
        Self {
            core: crate::core::Core::at(executable),
            root,
            calls,
        }
    }
    fn arguments(&self) -> Vec<String> {
        std::fs::read(&self.calls)
            .unwrap()
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8(arg.to_vec()).unwrap())
            .collect()
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn fixture() -> App {
    let mut app = crate::board::app::tests::crew(Default::default());
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.view.as_mut().unwrap().me_id = Some("ben-id".into());
    app
}
fn send(app: &mut App, fake: &Fake, text: &str) -> Completed {
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        app.input.as_ref().unwrap().compose,
        Compose::Talk { .. }
    ));
    assert_eq!(app.paste(text), Effect::None);
    let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) else {
        panic!("expected explicit send");
    };
    let ticket = app.outgoing_ticket(&request).unwrap();
    app.in_flight += 1;
    assert!(app.reload_blocked());
    let BoardEvent::Acted {
        outcome: Ok(ActionOutcome::Send(completed)),
        sends: true,
        ..
    } = lane::run(&fake.core, None, lane::Job::Send { request, ticket })
    else {
        panic!("expected typed send feedback");
    };
    *completed
}
fn finish(app: &mut App, completed: Completed) {
    app.in_flight = app.in_flight.saturating_sub(1);
    app.finished_send(completed);
}
fn draw(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal
        .draw(|frame| crate::board::conversation::render(frame, app, frame.area()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn five_dispositions_keep_delivery_separate_from_the_acquired_agent_state() {
    for (document, label) in [
        (json!({"requestId":"sent", "status":"sent"}), "✓ delivered"),
        (
            json!({"requestId":"queued", "status":"queued"}),
            "◌ queued · inbox",
        ),
        (
            json!({"requestId":"offline", "offline":true}),
            "○ offline · queued",
        ),
        (
            json!({"requestId":"held", "notification":"held"}),
            "◌ held · digest",
        ),
        (
            json!({"requestId":"uncertain", "deliveryState":"uncertain"}),
            "? unknown · do not resend",
        ),
    ] {
        let fake = Fake::new(document, false);
        let mut app = fixture();
        let completed = send(&mut app, &fake, "exact outgoing text");
        let sending = draw(&app);
        assert!(
            sending.contains("You · now")
                && sending.contains("exact outgoing text")
                && sending.contains("sending")
        );
        assert!(app.input.as_ref().unwrap().text.is_empty());
        finish(&mut app, completed);
        let painted = draw(&app);
        assert!(painted.contains(label), "{painted}");
        assert!(painted.contains("● auth-fix working"), "{painted}");
        assert!(!painted.contains("read by") && !painted.contains("seen by"));
        assert!(!app.composer.has_drafts());
        let arguments = fake.arguments();
        assert_eq!(
            arguments
                .iter()
                .filter(|arg| arg.as_str() == "talk")
                .count(),
            1
        );
        assert_eq!(arguments.last().unwrap(), "exact outgoing text");
        assert!(app.input.as_ref().unwrap().text.is_empty());
    }
}

#[test]
fn refusal_retains_the_draft_but_an_error_with_a_request_cannot_offer_resend() {
    for request in [None, Some("accepted-error")] {
        let mut error = json!({"error":{"code":"DELIVERY_FAILED", "message":"Transport stopped."}});
        if let Some(request) = request {
            error["requestId"] = json!(request);
        }
        let fake = Fake::new(error, true);
        let mut app = fixture();
        let completed = send(&mut app, &fake, "kept unless accepted");
        assert_eq!(
            completed.outcome.as_ref().unwrap_err().request.as_deref(),
            request
        );
        finish(&mut app, completed);
        let painted = draw(&app);
        if request.is_some() {
            assert!(app.input.as_ref().unwrap().text.is_empty());
            assert!(painted.contains("? unknown · do not resend"), "{painted}");
        } else {
            assert_eq!(app.input.as_ref().unwrap().text, "kept unless accepted");
            assert!(painted.contains("draft kept"), "{painted}");
        }
        assert_eq!(
            fake.arguments()
                .iter()
                .filter(|arg| arg.as_str() == "talk")
                .count(),
            1
        );
    }
}

#[test]
fn completion_uses_the_frozen_sequence_and_uuid_pair_across_selection_sender_and_rename() {
    let fake = Fake::new(json!({"requestId":"first", "status":"sent"}), false);
    let mut app = fixture();
    let first = send(&mut app, &fake, "alpha text");
    let first_identity = first.ticket.as_ref().unwrap().key.clone();
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.selected = 1;
    app.view.as_mut().unwrap().me_id = Some("another-sender".into());
    app.view.as_mut().unwrap().me = Some("Other".into());
    finish(&mut app, first.clone());
    assert!(!draw(&app).contains("alpha text"));
    app.view.as_mut().unwrap().me_id = Some(first_identity.0.clone());
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.selected = 0;
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["name"] = json!("renamed");
    assert!(draw(&app).contains("alpha text"));
    app.selected = 1;
    let second = send(&mut app, &fake, "beta text");
    app.finished_send(first);
    assert!(
        app.composer.has_drafts(),
        "duplicate old feedback cannot clear the new draft"
    );
    finish(&mut app, second);
    assert!(draw(&app).contains("beta text") && !draw(&app).contains("alpha text"));
    app.selected = 0;
    assert!(draw(&app).contains("alpha text") && !draw(&app).contains("beta text"));
}

#[test]
fn only_exact_request_and_originator_bodies_attach_even_when_another_reply_is_latest() {
    let fake = Fake::new(json!({"requestId":"accepted", "status":"sent"}), false);
    let mut app = fixture();
    let completed = send(&mut app, &fake, "question");
    finish(&mut app, completed);
    app.view.as_mut().unwrap().replies = vec![
        json!({"recipientId":"auth-fix", "requestId":"wrong", "response":"unrelated body", "submittedAtMs":200}),
    ];
    assert!(!draw(&app).contains("unrelated body"));
    app.view.as_mut().unwrap().replies.push(json!({"recipientId":"auth-fix", "requestId":"accepted", "response":"correlated answer", "submittedAtMs":100}));
    let painted = draw(&app);
    assert!(painted.contains("auth-fix · now") && painted.contains("correlated answer"));
    assert!(!painted.contains("unrelated body"));
    app.view.as_mut().unwrap().me_id = Some("different-sender".into());
    assert!(!draw(&app).contains("correlated answer"));
}

#[test]
fn outgoing_retention_is_bounded_and_keeps_follow_without_a_transcript_store() {
    let fake = Fake::new(json!({"requestId":"r", "status":"queued"}), false);
    let mut app = fixture();
    for index in 0..20 {
        if app.input.is_some() {
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        let completed = send(&mut app, &fake, &format!("message {index}"));
        finish(&mut app, completed);
    }
    let state = app.conversation.borrow();
    let session = state.sessions.values().next().unwrap();
    assert_eq!(session.outgoing.len(), MAX_ITEMS);
    assert!(session.outgoing.iter().map(Entry::bytes).sum::<usize>() <= MAX_BYTES);
    assert!(session.follow);
    drop(state);
    drop(app);
    assert!(fixture().conversation.borrow().sessions.is_empty());
}

#[test]
fn turns_have_distinct_roles_tight_status_and_short_notice_without_request_ids() {
    let fake = Fake::new(
        json!({"requestId":"req-private-display-id", "status":"sent"}),
        false,
    );
    let mut app = fixture();
    let completed = send(&mut app, &fake, "literal auth-fix · now");
    finish(&mut app, completed);
    assert_eq!(app.notice.as_deref(), Some("Sent to auth-fix"));
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":"auth-fix", "requestId":"req-private-display-id", "response":"answer body"
    })];
    let shown = draw(&app);
    assert!(!shown.contains("req-private-display-id"));
    assert!(!shown.contains("– · –"));
    let state = app.conversation.borrow();
    let session = state.sessions.values().next().unwrap();
    let rendered = session.rendered.as_ref().unwrap();
    let lines = &rendered.mapped.lines;
    let text: Vec<_> = lines.iter().map(ToString::to_string).collect();
    assert_eq!(
        text,
        [
            "You · now",
            "literal auth-fix · now",
            "✓ delivered · ● auth-fix working",
            "",
            "auth-fix · now",
            "answer body"
        ]
    );
    assert_eq!(lines[0].style, app.look().role(Role::Muted));
    assert_eq!(lines[2].style, app.look().role(Role::Muted));
    assert_eq!(lines[4].style, app.look().role(Role::Accent));
    assert_ne!(
        lines[1].style,
        app.look().role(Role::Accent),
        "dynamic text has no authored turn role"
    );
    assert_eq!(crate::board::glyph_guard::glyph_error(&text[2]), None);
}

#[test]
fn every_correlated_reply_has_one_blank_before_the_next_turn_after_source_repaint() {
    let mut app = fixture();
    for index in 0..3 {
        let request = format!("req-turn-{index}");
        let fake = Fake::new(json!({"requestId":request,"status":"sent"}), false);
        let completed = send(&mut app, &fake, &format!("message {index}"));
        finish(&mut app, completed);
        app.view.as_mut().unwrap().replies = vec![json!({
            "recipientId":"auth-fix","requestId":request,"response":format!("reply {index}")
        })];
        draw(&app);
    }
    for repaint in [false, true] {
        if repaint {
            for session in app.conversation.borrow_mut().sessions.values_mut() {
                session.rendered = None;
            }
        }
        draw(&app);
        let state = app.conversation.borrow();
        let session = state.sessions.values().next().unwrap();
        let rendered = session.rendered.as_ref().unwrap();
        let lines: Vec<_> = rendered
            .mapped
            .lines
            .iter()
            .map(ToString::to_string)
            .collect();
        for index in 0..3 {
            let message = lines
                .iter()
                .position(|line| line == &format!("message {index}"))
                .unwrap();
            assert!(lines[message + 1].starts_with("✓ delivered"), "{lines:?}");
            let reply = lines
                .iter()
                .position(|line| line == &format!("reply {index}"))
                .unwrap();
            assert_eq!(lines[reply - 1], "auth-fix · now", "{lines:?}");
            if index < 2 {
                assert_eq!(
                    &lines[reply + 1..reply + 3],
                    &["", "You · now"],
                    "repaint={repaint}; sources={:?}; compact={:?}; {lines:?}",
                    rendered.mapped.sources,
                    session.compact_separators
                );
            }
        }
    }
}
