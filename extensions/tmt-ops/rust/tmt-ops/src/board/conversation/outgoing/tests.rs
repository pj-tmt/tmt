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
use tmt_cli_style::Role;

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
fn begin(app: &mut App, text: &str) -> (Ticket, crate::board::app::Request) {
    app.perform(&crate::action::Action::parse("talk").unwrap());
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
    (ticket, request)
}
fn send(app: &mut App, fake: &Fake, text: &str) -> Completed {
    let (ticket, request) = begin(app, text);
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

#[test]
fn row_context_reaches_the_worker_and_outgoing_ticket_exactly_once() {
    for action in ["talk", "annotate lead", "annotate member"] {
        let mut app = fixture();
        app.view.as_mut().unwrap().document["squad"]["lead"] =
            json!({"id":"lead-id","name":"Lead"});
        app.selected = app
            .rows()
            .iter()
            .position(|(_, row)| row["id"] == "auth-fix")
            .unwrap();
        assert_eq!(app.selected_row().unwrap()["name"], "auth-fix");
        app.perform(&crate::action::Action::parse(action).unwrap());
        app.paste("  about this row\nkeep the exact comment  ");
        let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("{action}: explicit row message");
        };
        let expected = "[product · auth-fix]   about this row\nkeep the exact comment  ";
        let ticket = app.outgoing_ticket(&request).unwrap();
        assert_eq!(ticket.text, expected, "{action}: optimistic body");
        let fake = Fake::new(json!({"requestId":"context-note","status":"queued"}), false);
        let BoardEvent::Acted {
            outcome: Ok(ActionOutcome::Send(completed)),
            ..
        } = lane::run(&fake.core, None, lane::Job::Send { request, ticket })
        else {
            panic!("{action}: typed send result");
        };
        assert!(completed.outcome.is_ok());
        let args = fake.arguments();
        let recipient = if action == "annotate lead" {
            "Lead"
        } else {
            "auth-fix"
        };
        assert!(
            args.windows(2).any(|pair| pair == [recipient, expected]),
            "{action}: the frozen body reaches only the admitted recipient"
        );
        assert_eq!(
            args.iter().filter(|arg| *arg == expected).count(),
            1,
            "{action}: exact Core message operand"
        );
    }
}

#[test]
fn exact_passage_precedes_the_comment_in_the_delivered_row_message() {
    use crate::board::{response::Selection, row_detail::Target};
    const RAW: &str = "first \"raw\"\n\n漢字 e\u{301}\n";
    const EXPECTED: &str =
        "[product · auth-fix]\n> first \"raw\"\n>\n> 漢字 e\u{301}\n>\n\n  my comment\nnext  ";
    for action in ["talk", "annotate lead", "annotate member"] {
        let mut app = fixture();
        let view = app.view.as_mut().unwrap();
        view.document["squad"]["lead"] = json!({"id":"lead-id","name":"Lead"});
        view.replies =
            vec![json!({"recipientId":"auth-fix","requestId":"raw-source","response":RAW})];
        app.selected = app
            .rows()
            .iter()
            .position(|(_, row)| row["id"] == "auth-fix")
            .unwrap();
        app.perform(&crate::action::Action::parse(action).unwrap());
        let compose = app.input.as_ref().unwrap().compose.clone();
        let target = Target::Row(app.row_target(app.selected).unwrap());
        let reply = app.detail(&target).unwrap().reply.unwrap();
        assert!(reply.acquired && reply.source);
        assert_eq!(reply.body, RAW);
        let mut selection = Selection::new(reply.key.unwrap(), reply.body);
        selection.source.key(
            KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            80,
            usize::MAX,
        );
        let quote = selection.quote().unwrap();
        assert_eq!(quote.text, RAW);
        app.attach_response_quote(target, quote.clone())
            .finish(&mut app);
        assert_eq!(
            app.input.as_ref().unwrap().compose,
            compose,
            "attaching a source never retargets the admitted composer"
        );
        assert_eq!(app.input.as_ref().unwrap().response_quotes, [quote]);
        app.paste("  my comment\nnext  ");
        let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("{action}: exact source should remain admissible");
        };
        let ticket = app.outgoing_ticket(&request).unwrap();
        assert_eq!(ticket.text, EXPECTED, "{action}: exact optimistic body");
        let fake = Fake::new(json!({"requestId":"quoted-note","status":"queued"}), false);
        let BoardEvent::Acted {
            outcome: Ok(ActionOutcome::Send(completed)),
            ..
        } = lane::run(&fake.core, None, lane::Job::Send { request, ticket })
        else {
            panic!("{action}: typed send result");
        };
        assert!(completed.outcome.is_ok());
        let args = fake.arguments();
        let recipient = if action == "annotate lead" {
            "Lead"
        } else {
            "auth-fix"
        };
        assert!(
            args.windows(2).any(|pair| pair == [recipient, EXPECTED]),
            "{action}: the frozen body reaches only the admitted recipient"
        );
        assert_eq!(
            args.iter().filter(|arg| *arg == EXPECTED).count(),
            1,
            "{action}: literal delivered body"
        );
    }
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
fn five_dispositions_preserve_delivery_and_never_infer_activity_from_task_state() {
    for (document, label) in [
        (
            json!({"requestId":"sent", "status":"sent"}),
            "✓ sent · auth-fix's activity is not reported",
        ),
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
        // R1: `sending…` appears only after 400 ms.
        let started = crate::status::now_ms();
        let sending = crate::status::with_now_ms(started + 500, || draw(&app));
        assert!(
            sending.contains("You")
                && sending.contains("exact outgoing text")
                && sending.contains("sending")
        );
        assert!(app.input.as_ref().unwrap().text.is_empty());
        finish(&mut app, completed);
        // The visible `sending…` row is kept for 400 ms before the outcome.
        let painted = crate::status::with_now_ms(started + 6_000, || draw(&app));
        assert!(painted.contains(label), "{painted}");
        assert!(painted.contains("working · product · member"), "{painted}");
        assert!(
            !painted.contains("is working"),
            "the row's task state is not a provider observation: {painted}"
        );
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
        assert_eq!(
            arguments.last().unwrap(),
            "[product · auth-fix] exact outgoing text"
        );
        assert!(app.input.as_ref().unwrap().text.is_empty());
    }
}

fn provider_observation(app: &mut App, phase: crate::board::activity::Phase, since_ms: u64) {
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot
        .roster
        .members
        .get_mut(crate::board::app::workspace::tests::MEMBER)
        .unwrap()
        .activity = crate::board::activity::ObservedActivity {
        phase,
        since_ms: Some(since_ms),
        last_ms: Some(since_ms),
    };
}

#[test]
fn sent_feedback_uses_provider_time_even_when_request_attention_and_task_disagree() {
    use crate::board::{activity::Phase, app::workspace::tests::MEMBER};
    const SENT: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(SENT, || {
        for (since, expected) in [
            (SENT, "⠹ worker is working · 0:42"),
            (SENT - 180_000, "◌ worker is finishing an earlier turn · 3m"),
        ] {
            let mut app = crate::board::app::workspace::tests::board();
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .reduced_motion = true;
            let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
            let membership = snapshot.roster.squads[0]
                .member_states
                .get_mut(MEMBER)
                .unwrap();
            membership.waiting = true;
            membership.state = Some("blocked".into());
            provider_observation(&mut app, Phase::Working, since);
            let fake = Fake::new(json!({"requestId":"provider-time","status":"sent"}), false);
            let completed = send(&mut app, &fake, "one explicit message");
            finish(&mut app, completed);
            let _no_acquisition = crate::runner::forbid_commands();
            let screen = crate::status::with_now_ms(SENT + 42_000, || draw(&app));
            assert!(screen.contains(expected), "{screen}");
            assert!(screen.contains("waiting · blocked · product"), "{screen}");
            assert_eq!(
                app.conversation
                    .borrow()
                    .sessions
                    .values()
                    .next()
                    .unwrap()
                    .outgoing
                    .len(),
                1
            );
        }
    });
}

#[test]
fn observed_completion_then_exact_reply_replaces_feedback_without_another_send() {
    use crate::board::{activity::Phase, app::workspace::tests::MEMBER};
    const SENT: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(SENT, || {
        let mut app = crate::board::app::workspace::tests::board();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .reduced_motion = true;
        provider_observation(&mut app, Phase::Working, SENT);
        let fake = Fake::new(
            json!({"requestId":"provider-completion","status":"sent"}),
            false,
        );
        let completed = send(&mut app, &fake, "one explicit message");
        finish(&mut app, completed);
        assert!(
            crate::status::with_now_ms(SENT + 42_000, || draw(&app))
                .contains("⠹ worker is working · 0:42")
        );
        provider_observation(&mut app, Phase::Idle, SENT + 60_000);
        let _no_acquisition = crate::runner::forbid_commands();
        assert!(
            crate::status::with_now_ms(SENT + 180_000, || draw(&app))
                .contains("✓ worker finished without replying · 2m ago")
        );
        app.view.as_mut().unwrap().replies.push(json!({
            "recipientId":MEMBER,"requestId":"provider-completion", "response":"Exact acquired reply"
        }));
        let screen = crate::status::with_now_ms(SENT + 181_000, || draw(&app));
        assert!(screen.contains("Exact acquired reply"), "{screen}");
        assert!(
            !screen.contains("finished without replying") && !screen.contains("is working"),
            "{screen}"
        );
        let state = app.conversation.borrow();
        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.outgoing.len(), 1);
        let key = session.outgoing[0]
            .reply
            .as_ref()
            .unwrap()
            .key
            .as_ref()
            .unwrap();
        assert_eq!(key.request, "provider-completion");
        assert_eq!(
            session.outgoing[0].reply.as_ref().unwrap().body,
            "Exact acquired reply"
        );
        assert_eq!(
            fake.arguments()
                .iter()
                .filter(|argument| argument.as_str() == "talk")
                .count(),
            1
        );
    });
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
    assert!(draw(&app).contains("unrelated body"));
    assert!(
        app.conversation
            .borrow()
            .sessions
            .values()
            .next()
            .unwrap()
            .outgoing[0]
            .reply
            .is_none(),
        "another request's final is visible history, never attached to this local turn"
    );
    app.view.as_mut().unwrap().replies.push(json!({"recipientId":"auth-fix", "requestId":"accepted", "response":"correlated answer", "submittedAtMs":100}));
    let painted = draw(&app);
    assert!(painted.contains("auth-fix") && painted.contains("correlated answer"));
    assert!(painted.contains("unrelated body"));
    assert_eq!(
        app.conversation
            .borrow()
            .sessions
            .values()
            .next()
            .unwrap()
            .outgoing[0]
            .reply
            .as_ref()
            .unwrap()
            .body,
        "correlated answer"
    );
    app.view.as_mut().unwrap().me_id = Some("different-sender".into());
    // A new actor's snapshot has its own originator results page.
    app.view.as_mut().unwrap().replies.clear();
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
    assert_eq!(
        session.outgoing.len(),
        20,
        "no send is dropped below the cap"
    );
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
    // Past the evidence grace period an agent without activity says so.
    crate::status::with_now_ms(crate::status::now_ms() + 6_000, || draw(&app));
    {
        let state = app.conversation.borrow();
        let lines = &state
            .sessions
            .values()
            .next()
            .unwrap()
            .rendered
            .as_ref()
            .unwrap()
            .mapped
            .lines;
        assert_eq!(
            lines[2].to_string(),
            "✓ sent · auth-fix's activity is not reported"
        );
        assert_eq!(lines[2].style, app.look().role(Role::Muted));
    }
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
        &text[..3],
        [
            "You · now",
            "[product · auth-fix] literal auth-fix · now",
            ""
        ]
    );
    assert!(
        text[3].contains("auth-fix")
            && text[3].contains("→")
            && text[3].contains("Ben")
            && text[3].contains('—'),
        "unknown settled time is not invented: {text:?}"
    );
    assert_eq!(text[4], "answer body");
    assert_eq!(lines[0].style, app.look().role(Role::Muted));
    assert!(
        lines[3]
            .spans
            .iter()
            .any(|span| span.content.contains("auth-fix")
                && span.style.fg == app.look().role(Role::Text).fg)
    );
    assert!(
        session
            .authored_lines
            .values()
            .all(|line| !matches!(line, super::super::AuthoredLine::Status { .. }))
    );
    assert_ne!(
        lines[1].style,
        app.look().role(Role::Accent),
        "dynamic text has no authored turn role"
    );
    assert_eq!(
        crate::board::glyph_guard::glyph_error(&text.join("\n")),
        None
    );
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
                .position(|line| line == &format!("[product · auth-fix] message {index}"))
                .unwrap();
            assert_eq!(lines[message + 1], "", "{lines:?}");
            assert_eq!(
                lines[message + 3],
                format!("reply {index}"),
                "the correlated reply immediately replaces feedback: {lines:?}"
            );
            let reply = lines
                .iter()
                .position(|line| line.trim().trim_start_matches("│ ") == format!("reply {index}"))
                .unwrap();
            assert!(
                lines[reply - 1].contains("auth-fix") && lines[reply - 1].contains('→'),
                "{lines:?}"
            );
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

#[test]
fn a_replied_outgoing_body_renders_in_full_beside_its_reply_and_the_next_turn() {
    let mut app = fixture();
    let message = (0..16)
        .map(|n| format!("outgoing line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let fake = Fake::new(json!({"requestId":"long-turn","status":"sent"}), false);
    let completed = send(&mut app, &fake, &message);
    finish(&mut app, completed);
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":"auth-fix","requestId":"long-turn","response":"Exact final remains visible"
    })];
    draw(&app);
    let next = Fake::new(json!({"requestId":"next-turn","status":"sent"}), false);
    let completed = send(&mut app, &next, "Next turn remains visible");
    finish(&mut app, completed);
    let shown = draw(&app);
    assert!(!shown.contains("more lines"), "{shown}");
    assert!(shown.contains("outgoing line 15"), "{shown}");
    assert!(shown.contains("Exact final remains visible"), "{shown}");
    assert!(shown.contains("Next turn remains visible"), "{shown}");
    let state = app.conversation.borrow();
    let session = state.sessions.values().next().unwrap();
    assert!(session.outgoing[0].reply.is_some());
    assert!(session.outgoing[1].reply.is_none());
}

#[test]
fn delivery_and_acquired_agent_freshness_remain_independent_after_failed_reads() {
    use crate::board::workspace::OpsObservation;
    const SENT: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(SENT, || {
        for source in [
            "roster",
            "squad",
            "membership",
            "read-error",
            "read-age",
            "document",
        ] {
            let mut app = crate::board::app::workspace::tests::board();
            let view = app.view.as_mut().unwrap();
            view.look.depth = tmt_cli_style::Depth::None;
            view.home.as_mut().unwrap().workspace.squad_looks.clear();
            provider_observation(&mut app, crate::board::activity::Phase::Working, SENT);
            let fake = Fake::new(json!({"requestId":"req-freshness","status":"sent"}), false);
            let completed = send(&mut app, &fake, "one explicit message");
            finish(&mut app, completed);
            let _no_acquisition = crate::runner::forbid_commands();
            let first = board_frame(&app, 148, 51);
            assert!(
                first.contains("⠹ worker is working · 0:00"),
                "{source}: {first}"
            );
            assert_eq!(feedback_marks(&app, SENT).len(), 1);
            match source {
                "roster" => {
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .roster
                        .observation = OpsObservation::Stale
                }
                "squad" => {
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .roster
                        .squads[0]
                        .observation = OpsObservation::Partial
                }
                "membership" => {
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .roster
                        .membership_complete = false
                }
                "read-error" => app.error = Some("retained after failed read".into()),
                "read-age" => {
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .read_at =
                        Some(std::time::Instant::now() - std::time::Duration::from_secs(3600))
                }
                "document" => app.view.as_mut().unwrap().document["partial"] = json!(true),
                _ => unreachable!(),
            }
            let screen = crate::status::with_now_ms(crate::status::now_ms() + 6_000, || {
                board_frame(&app, 148, 51)
            });
            assert!(
                screen.contains("worker's activity is not reported"),
                "{source}: {screen}"
            );
            assert!(!screen.contains("worker is working"), "{source}: {screen}");
            assert!(feedback_marks(&app, SENT).is_empty());
            assert_eq!(crate::board::view::feedback_wait(&app, SENT), None);
            let state = app.conversation.borrow();
            let session = state.sessions.values().next().unwrap();
            assert!(
                session.outgoing[0].status() == "✓ sent · worker's activity is not reported"
                    && !session.text.contains("is working"),
                "{source}: missing provider evidence stays unknown: {}",
                session.text
            );
            assert!(
                matches!(
                    &session.outgoing.front().unwrap().outcome,
                    Some(Outcome::Send(Ok(Accepted {
                        disposition: Disposition::Sent,
                        ..
                    })))
                ),
                "{source}: delivery stays the typed acceptance fact"
            );
            assert_eq!(session.outgoing.len(), 1, "a failed read never resends");
        }
    });
}

#[test]
fn outgoing_byte_budget_evicts_old_turns_and_late_completion_keeps_new_edits() {
    let mut app = fixture();
    let payload = "x".repeat(crate::board::app::MESSAGE_LIMIT - "[product · auth-fix] ".len());
    let mut oldest = None;
    for _ in 0..6 {
        if app.input.is_some() {
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.paste(&payload);
        let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("expected explicit send");
        };
        let ticket = app.outgoing_ticket(&request).unwrap();
        assert_eq!(ticket.text, format!("[product · auth-fix] {payload}"));
        assert_eq!(ticket.text.len(), crate::board::app::MESSAGE_LIMIT);
        let completed = Completed {
            ticket: Some(ticket),
            outcome: Ok(Accepted {
                request: format!("budget-request-{}", app.conversation.borrow().sequence),
                disposition: Disposition::Queued,
            }),
            what: "Message".into(),
            to: "worker".into(),
        };
        oldest.get_or_insert_with(|| completed.clone());
        finish(&mut app, completed);
    }
    let old = oldest.unwrap();
    let state = app.conversation.borrow();
    let ticket = old.ticket.as_ref().unwrap();
    let session = &state.sessions[&(Some(ticket.key.0.clone()), ticket.key.1.clone())];
    assert_eq!(
        session.outgoing.len(),
        MAX_BYTES / crate::board::app::MESSAGE_LIMIT
    );
    assert_eq!(
        session.outgoing.iter().map(Entry::bytes).sum::<usize>(),
        MAX_BYTES
    );
    assert!(
        session
            .outgoing
            .iter()
            .all(|entry| entry.ticket.sequence > old.ticket.as_ref().unwrap().sequence)
    );
    drop(state);
    app.paste("newer draft must survive");
    let notice = app.notice.clone();
    app.finished_send(old);
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        "newer draft must survive"
    );
    assert_eq!(app.notice, notice);
}

#[test]
fn outgoing_preserves_exact_effect_bytes_without_painting_terminal_controls() {
    let fake = Fake::new(
        json!({"requestId":"controls-request", "status":"queued"}),
        false,
    );
    let mut app = fixture();
    let text = "You · now\nalpha · now\nraw\u{1b}[2J\u{7} 漢字 e\u{301}";
    let delivered = format!("[product · auth-fix] {text}");
    let completed = send(&mut app, &fake, text);
    assert!(
        fake.arguments()
            .iter()
            .any(|argument| argument == &delivered)
    );
    let state = app.conversation.borrow();
    let ticket = completed.ticket.as_ref().unwrap();
    let session = &state.sessions[&(Some(ticket.key.0.clone()), ticket.key.1.clone())];
    assert_eq!(session.outgoing.back().unwrap().ticket.text, delivered);
    drop(state);
    finish(&mut app, completed);
    let painted = draw(&app);
    assert!(
        painted.contains('漢') && painted.contains('字'),
        "{painted:?}"
    );
    assert!(!painted.contains("delivered"));
    assert!(!painted.contains('\u{1b}') && !painted.contains('\u{7}'));
    assert!(painted.contains("queued · inbox"));
}

#[test]
fn answers_use_the_existing_worker_once_and_keep_the_question_and_reply_in_one_group() {
    use crate::board::{
        app::workspace::tests::{board, observe_waiting_member},
        shell::OpsTab,
    };
    for failure in [false, true] {
        let mut app = board();
        observe_waiting_member(&mut app);
        app.shell.tab = OpsTab::Agents;
        let target = app.workspace.selected.clone().unwrap();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .rows
            .get_mut(&target)
            .unwrap()["waitingOnYou"] = json!([{"requestId":"req_99999999-9999-4999-8999-999999999999","preview":"Question retained","preparedAtMs":crate::status::now_ms()-1_000}]);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            Compose::Talk { .. }
        ));
        app.paste("Exact answer");
        let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("explicit answer");
        };
        let expected = match &request {
            Request::Reply { request, .. } => request.clone(),
            _ => panic!("answer owns request identity"),
        };
        let ticket = app.outgoing_ticket(&request).unwrap();
        let fake = Fake::new(
            if failure {
                json!({"error":{"code":"REFUSED","message":"Not submitted."}})
            } else {
                json!({"status":"submitted","requestId":expected})
            },
            failure,
        );
        let BoardEvent::Acted {
            outcome: Ok(ActionOutcome::Answer(completed)),
            sends: true,
            ..
        } = lane::run(&fake.core, None, lane::Job::Send { request, ticket })
        else {
            panic!("typed answer completion");
        };
        assert_eq!(completed.outcome.is_err(), failure);
        let frozen = completed.ticket.clone();
        app.finished_answer(*completed);
        assert_eq!(
            fake.arguments(),
            [
                "answer",
                "--identity",
                "Owner",
                "--request",
                &expected,
                "--json",
                "--",
                "worker",
                "[product · worker] Exact answer"
            ]
        );
        let state = app.conversation.borrow();
        let session = &state.sessions[&(Some(frozen.key.0), frozen.key.1)];
        assert!(
            session.text.contains("Question retained") && session.text.contains("Exact answer")
        );
        assert!(session.text.contains(if failure {
            "reply failed"
        } else {
            "✓ replied"
        }));
        drop(state);
        assert_eq!(app.composer.has_drafts(), failure);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            app.reload_blocked(),
            failure,
            "only the retained failed draft blocks reload after leaving the editor"
        );
    }
}

#[test]
fn a_new_incoming_question_does_not_discard_an_unsettled_send() {
    let mut app = fixture();
    app.perform(&crate::action::Action::parse("talk").unwrap());
    app.paste("in flight");
    let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) else {
        panic!("send");
    };
    let ticket = app.outgoing_ticket(&request).unwrap();
    let identity = (Some(ticket.key.0.clone()), ticket.key.1.clone());
    let incoming = Reply {
        from: "auth-fix".into(),
        age: "now".into(),
        at_ms: None,
        body: "New exact question".into(),
        key: Some(crate::board::home_leads::MessageKey {
            sender: ticket.key.0.clone(),
            lead: ticket.key.1.clone(),
            squad: "product".into(),
            request: "incoming".into(),
            kind: crate::board::home_leads::Kind::Question,
        }),
        acquired: true,
        source: false,
    };
    app.conversation
        .borrow_mut()
        .sessions
        .get_mut(&identity)
        .unwrap()
        .update(Some(incoming.clone()));
    app.finished_send(Completed {
        ticket: Some(ticket),
        outcome: Ok(Accepted {
            request: "sent".into(),
            disposition: Disposition::Queued,
        }),
        what: "Message".into(),
        to: "auth-fix".into(),
    });
    assert!(!app.composer.has_drafts());
    app.conversation
        .borrow_mut()
        .sessions
        .get_mut(&identity)
        .unwrap()
        .update(Some(incoming));
    let text = app.conversation.borrow().sessions[&identity].text.clone();
    assert!(
        text.starts_with("You · now\n\n") && text.contains("in flight"),
        "the settled send stays: {text:?}"
    );
    assert!(
        text.ends_with("auth-fix · now\n\nNew exact question"),
        "the new question is a later block below it: {text:?}"
    );
}

#[test]
fn header_shows_core_activity_and_feedback_never_infers_it_from_task_state() {
    use crate::board::app::workspace::tests::{MEMBER, ROOM};
    for (waiting, words) in [(false, "working · blocked"), (true, "waiting · blocked")] {
        let mut app = crate::board::app::workspace::tests::board();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        let membership = snapshot.roster.squads[0]
            .member_states
            .get_mut(MEMBER)
            .unwrap();
        membership.state = Some("blocked".into());
        membership.waiting = waiting;
        let target = crate::board::workspace::OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        };
        snapshot.rows.get_mut(&target).unwrap()["state"] = json!("blocked");
        app.workspace_reconcile();
        let fake = Fake::new(json!({"requestId":"req-mismatch","status":"sent"}), false);
        let completed = send(&mut app, &fake, "one explicit message");
        finish(&mut app, completed);
        let _no_acquisition = crate::runner::forbid_commands();
        let screen = draw(&app);
        assert!(
            screen.contains(&format!("{words} · product")),
            "header: {screen}"
        );
        assert!(
            screen.contains("✓ sent · worker's activity is not reported"),
            "task words do not prove provider progress: {screen}"
        );
        assert!(
            !screen.contains("✗ worker") && !screen.contains("worker is working"),
            "the task never picks the feedback glyph or phase: {screen}"
        );
    }
}

#[test]
fn a_settled_answer_falls_back_once_and_keeps_the_actual_talk_acceptance() {
    use crate::board::app::workspace::tests::board;
    for uncertain in [false, true] {
        let mut app = board();
        let target = app.workspace.selected.clone().unwrap();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .rows
            .get_mut(&target)
            .unwrap()["waitingOnYou"] = json!([
            {"requestId":"settled-request", "preparedAtMs":20}
        ]);
        app.focus_composer();
        let text = "Keep the exact first line.\nAnd the second.";
        app.paste(text);
        let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("member message must dispatch");
        };
        let ticket = app.outgoing_ticket(&request).unwrap();
        let expected = ticket.text.clone();
        let fake = Fake::new(json!({}), false);
        let document = if uncertain {
            json!({"requestId":"accepted-fallback", "error":{"code":"TALK_DELIVERY_FAILED", "message":"Delivery unknown."}})
        } else {
            json!({"requestId":"accepted-fallback", "status":"queued"})
        };
        let document = document.to_string().replace('\'', "'\"'\"'");
        let script = format!(
            r#"#!/bin/sh
printf '%s\0' "$@" >> '{calls}'
if [ "$1" = answer ]; then
  printf '%s\n' '{{"error":{{"code":"RESPONSE_CONFLICT","message":"Settled meanwhile."}}}}'
  exit 1
fi
printf '%s\n' '{document}'
exit {status}
"#,
            calls = fake.calls.display(),
            status = usize::from(uncertain)
        );
        crate::test_support::write_ready_executable(&fake.root.join("tmt"), &script);
        let BoardEvent::Acted {
            outcome: Ok(ActionOutcome::Send(completed)),
            sends: true,
            ..
        } = lane::run(&fake.core, None, lane::Job::Send { request, ticket })
        else {
            panic!("the fallback must retain Talk completion, not answer completion");
        };
        assert!(
            completed.accepted(),
            "retained acceptance must not offer a resend"
        );
        if uncertain {
            assert_eq!(
                completed.outcome.as_ref().unwrap_err().request.as_deref(),
                Some("accepted-fallback")
            );
        } else {
            assert_eq!(
                completed.outcome.as_ref().unwrap().request,
                "accepted-fallback"
            );
        }
        app.finished_send(*completed);
        assert!(app.input.as_ref().unwrap().text.is_empty());
        let args = fake.arguments();
        assert_eq!(
            args.iter().filter(|arg| arg.as_str() == "answer").count(),
            1
        );
        assert_eq!(args.iter().filter(|arg| arg.as_str() == "talk").count(), 1);
        assert_eq!(
            args.iter().filter(|arg| *arg == &expected).count(),
            2,
            "both operations receive the identical multiline body"
        );
        assert!(app.notice.as_deref().unwrap().contains(if uncertain {
            "do not resend"
        } else {
            "Queued"
        }));
    }
}

fn board_frame(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
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

fn feedback_marks(app: &App, now_ms: u64) -> Vec<String> {
    app.frame_clocks
        .borrow()
        .iter()
        .filter_map(|surface| match surface {
            crate::board::view::ClockSurface::OutgoingFeedback(clock) => Some(clock.mark(now_ms)),
            _ => None,
        })
        .collect()
}

#[test]
fn only_painted_feedback_animates_and_covered_or_scrolled_out_feedback_stops() {
    const SENT: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(SENT, || {
        let mut app = crate::board::app::workspace::tests::board();
        app.view.as_mut().unwrap().look.depth = tmt_cli_style::Depth::TrueColor;
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .squad_looks
            .clear();
        provider_observation(&mut app, crate::board::activity::Phase::Working, SENT);
        let fake = Fake::new(json!({"requestId":"visible-motion","status":"sent"}), false);
        let body = (0..90)
            .map(|n| format!("outgoing line {n}\n\n"))
            .collect::<String>();
        let completed = send(&mut app, &fake, &body);
        finish(&mut app, completed);
        let _no_acquisition = crate::runner::forbid_commands();
        let screen = board_frame(&app, 148, 51);
        assert!(screen.contains("⠋ worker is working · 0:00"), "{screen}");
        let first = feedback_marks(&app, SENT);
        assert_eq!(first.len(), 1, "wrapped status admits one clock");
        assert_ne!(first, feedback_marks(&app, SENT + 80));
        assert!(crate::board::view::feedback_wait(&app, SENT).is_some());
        let animated = crate::status::with_now_ms(SENT + 80, || board_frame(&app, 148, 51));
        assert!(
            animated.contains("⠙ worker is working · 0:00"),
            "{animated}"
        );
        let screen = crate::status::with_now_ms(SENT + 1_000, || board_frame(&app, 148, 51));
        assert!(screen.contains("worker is working · 0:01"), "{screen}");
        app.help = true;
        board_frame(&app, 148, 51);
        assert!(feedback_marks(&app, SENT).is_empty());
        assert_eq!(crate::board::view::feedback_wait(&app, SENT), None);
        app.help = false;
        app.scroll_pane(
            crate::config::Pane::Replies,
            crate::board::scroll::Step::Top,
        );
        let screen = board_frame(&app, 148, 51);
        assert!(screen.contains("outgoing line 0"), "{screen}");
        assert!(!screen.contains("worker is working"), "{screen}");
        assert!(feedback_marks(&app, SENT).is_empty());
        assert_eq!(crate::board::view::feedback_wait(&app, SENT), None);
        app.scroll_pane(
            crate::config::Pane::Replies,
            crate::board::scroll::Step::Bottom,
        );
        assert!(board_frame(&app, 148, 51).contains("worker is working"));
        assert_eq!(feedback_marks(&app, SENT).len(), 1);
        assert_eq!(
            fake.arguments().iter().filter(|arg| *arg == "talk").count(),
            1
        );
    });
}

#[test]
fn reduced_motion_and_no_color_keep_static_glyph_and_live_elapsed_text() {
    const SENT: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(SENT, || {
        for no_color in [false, true] {
            let mut app = crate::board::app::workspace::tests::board();
            let view = app.view.as_mut().unwrap();
            let home = view.home.as_mut().unwrap();
            home.reduced_motion = !no_color;
            if no_color {
                home.workspace.squad_looks.clear();
                view.look.depth = tmt_cli_style::Depth::None;
            }
            provider_observation(&mut app, crate::board::activity::Phase::Working, SENT);
            let fake = Fake::new(json!({"requestId":"static-motion","status":"sent"}), false);
            let completed = send(&mut app, &fake, "one static status");
            finish(&mut app, completed);
            let _no_acquisition = crate::runner::forbid_commands();
            let screen = board_frame(&app, 148, 51);
            assert!(screen.contains("⠹ worker is working · 0:00"), "{screen}");
            let first = feedback_marks(&app, SENT);
            assert_eq!(first.len(), 1);
            assert_eq!(first, feedback_marks(&app, SENT + 80));
            assert_ne!(first, feedback_marks(&app, SENT + 1_000));
            assert_eq!(crate::board::view::feedback_wait(&app, SENT), None);
            let screen = crate::status::with_now_ms(SENT + 1_000, || board_frame(&app, 148, 51));
            assert!(screen.contains("⠹ worker is working · 0:01"), "{screen}");
        }
    });
}

#[test]
fn disabled_sampling_still_supplies_the_selected_rooms_motion_preference() {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().look.depth = tmt_cli_style::Depth::TrueColor;
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    let target = app.conversation_target().unwrap();
    let view = app.view.as_mut().unwrap();
    let home = view.home.as_mut().unwrap();
    home.reduced_motion = true;
    let squad = home.workspace.roster.squads[0].name.clone();
    assert!(!motion_allowed(&app, &target));
    app.view.as_mut().unwrap().home_rate.insert(
        squad.clone(),
        crate::board::app::RateView {
            settings: crate::config::TokenRate {
                enabled: false,
                reduced_motion: false,
                ..Default::default()
            },
            input: crate::board::rate::tests::input(0),
            history: None,
        },
    );
    assert!(motion_allowed(&app, &target));
    app.view
        .as_mut()
        .unwrap()
        .home_rate
        .get_mut(&squad)
        .unwrap()
        .settings
        .reduced_motion = true;
    assert!(!motion_allowed(&app, &target));
    let crate::board::app::RowTarget::Workspace(crate::board::workspace::OpsTarget::Member {
        identity_id,
        ..
    }) = target
    else {
        panic!("member target");
    };
    let common =
        crate::board::app::RowTarget::Workspace(crate::board::workspace::OpsTarget::Member {
            identity_id,
            room_id: None,
        });
    assert!(
        !motion_allowed(&app, &common),
        "common pool uses the global preference"
    );
}

#[test]
fn unknown_and_correlated_replies_never_admit_a_feedback_clock() {
    const SENT: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(SENT, || {
        let mut app = crate::board::app::workspace::tests::board();
        let fake = Fake::new(json!({"requestId":"unknown-clock","status":"sent"}), false);
        let completed = send(&mut app, &fake, "one explicit message");
        finish(&mut app, completed);
        let _no_acquisition = crate::runner::forbid_commands();
        let screen = board_frame(&app, 148, 51);
        assert!(
            screen.contains("worker's activity is not reported"),
            "{screen}"
        );
        assert!(feedback_marks(&app, SENT).is_empty());
        assert_eq!(crate::board::view::feedback_wait(&app, SENT), None);
        provider_observation(&mut app, crate::board::activity::Phase::Working, SENT);
        board_frame(&app, 148, 51);
        assert_eq!(feedback_marks(&app, SENT).len(), 1);
        app.view.as_mut().unwrap().replies.push(json!({
            "recipientId":crate::board::app::workspace::tests::MEMBER,
            "requestId":"unknown-clock", "response":"This exact reply replaces feedback"
        }));
        let screen = board_frame(&app, 148, 51);
        assert!(
            screen.contains("This exact reply replaces feedback"),
            "{screen}"
        );
        assert!(!screen.contains("worker is working"), "{screen}");
        assert!(feedback_marks(&app, SENT).is_empty());
        assert_eq!(crate::board::view::feedback_wait(&app, SENT), None);
    });
}

#[test]
fn a_visible_wrapped_continuation_keeps_elapsed_text_without_a_hidden_spinner_wakeup() {
    const SENT: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(SENT, || {
        let mut app = crate::board::app::workspace::tests::board();
        app.view.as_mut().unwrap().look.depth = tmt_cli_style::Depth::TrueColor;
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .squad_looks
            .clear();
        provider_observation(&mut app, crate::board::activity::Phase::Working, SENT);
        let fake = Fake::new(
            json!({"requestId":"clipped-spinner","status":"sent"}),
            false,
        );
        let completed = send(&mut app, &fake, "one outgoing message");
        finish(&mut app, completed);
        let surface = crate::board::conversation::exchange::Surface {
            target: app.conversation_target().unwrap(),
            interactive: true,
        };
        let paint = |width, height| {
            app.frame_clocks.borrow_mut().clear();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    crate::board::conversation::exchange::render(
                        frame,
                        &app,
                        frame.area(),
                        &surface,
                    )
                })
                .unwrap();
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
        };
        let _no_acquisition = crate::runner::forbid_commands();
        let narrow = paint(12, 3);
        assert!(narrow.contains("0:00"), "{narrow}");
        assert!(!narrow.contains('⠋'), "{narrow}");
        let first = feedback_marks(&app, SENT);
        assert_eq!(first.len(), 1);
        assert_eq!(first, feedback_marks(&app, SENT + 80));
        assert_ne!(first, feedback_marks(&app, SENT + 1000));
        assert_eq!(crate::board::view::feedback_wait(&app, SENT), None);
        let wide = paint(80, 8);
        assert!(wide.contains("⠋ worker is working · 0:00"), "{wide}");
        assert!(crate::board::view::feedback_wait(&app, SENT).is_some());
        assert_eq!(feedback_marks(&app, SENT).len(), 1);
    });
}

pub(in crate::board) mod captures;

#[test]
fn grouped_outgoing_turns_keep_an_actual_earlier_reply_heading_and_its_age() {
    use crate::board::app::workspace::tests::MEMBER;
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        let mut app = crate::board::app::workspace::tests::board();
        app.view.as_mut().unwrap().replies.push(json!({
            "recipientId":MEMBER,"requestId":"older-retained",
            "submittedAtMs":NOW-120_000,"response":"Older retained exact reply"
        }));
        let fake = Fake::new(json!({"requestId":"new-message","status":"sent"}), false);
        let completed = send(&mut app, &fake, "A new explicit message");
        finish(&mut app, completed);
        let _no_acquisition = crate::runner::forbid_commands();
        app.view.as_mut().unwrap().replies.push(json!({
            "recipientId":MEMBER,"requestId":"new-message",
            "submittedAtMs":NOW,"response":"New exact acquired reply"
        }));
        let screen = board_frame(&app, 148, 51);
        assert!(screen.contains("worker → Owner"), "{screen}");
        assert!(
            screen.contains("2m ago"),
            "the retained root keeps its own age: {screen}"
        );
        assert!(
            screen.contains("Older retained exact reply")
                && screen.contains("New exact acquired reply"),
            "{screen}"
        );
        let state = app.conversation.borrow();
        let session = state.sessions.values().next().unwrap();
        let held = session.before_send.as_ref().unwrap();
        let key = held.key.as_ref().unwrap();
        assert_eq!(key.request, "older-retained");
        assert_eq!(held.body, "Older retained exact reply");
        assert_eq!(
            fake.arguments().iter().filter(|arg| *arg == "talk").count(),
            1
        );
    });
}

#[test]
fn provider_feedback_names_are_literal_and_do_not_create_authored_member_links() {
    use crate::board::app::workspace::tests::MEMBER;
    const SENT: u64 = 1_791_600_000_000;
    const NAME: &str = "*night-2* #tmt:worker";
    crate::status::with_now_ms(SENT, || {
        let mut app = crate::board::app::workspace::tests::board();
        let view = app.view.as_mut().unwrap();
        view.look.depth = tmt_cli_style::Depth::None;
        view.home.as_mut().unwrap().workspace.squad_looks.clear();
        provider_observation(&mut app, crate::board::activity::Phase::Working, SENT);
        let fake = Fake::new(json!({"requestId":"literal-name","status":"sent"}), false);
        let completed = send(&mut app, &fake, "**Authored bold** #tmt:worker");
        finish(&mut app, completed);
        let target = app.workspace.selected.clone().unwrap();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot.rows.get_mut(&target).unwrap()["name"] = json!(NAME);
        snapshot.roster.members.get_mut(MEMBER).unwrap().name = NAME.into();
        app.references = crate::board::inventory::Inventory::from_listed(
            &[],
            &Ok(json!({"identities":[
                {"id":MEMBER,"name":NAME,"canonicalName":NAME},
                {"id":"99999999-9999-4999-8999-999999999999","name":"worker","canonicalName":"worker"}
            ]})),
        );
        let _no_acquisition = crate::runner::forbid_commands();
        let screen = crate::status::with_now_ms(SENT + 42_000, || board_frame(&app, 148, 51));
        assert!(
            screen.contains("⠹ *night-2* #tmt:worker is working · 0:42"),
            "{screen}"
        );
        {
            let state = app.conversation.borrow();
            let session = state.sessions.values().next().unwrap();
            let mapped = &session.rendered.as_ref().unwrap().mapped;
            assert_eq!(
                mapped.mentions.iter().map(Vec::len).sum::<usize>(),
                1,
                "only the authored body creates a reference, never generated names"
            );
            assert!(
                mapped
                    .lines
                    .iter()
                    .any(|line| line.to_string().contains("Authored bold worker")
                        && line
                            .spans
                            .iter()
                            .filter(|span| span
                                .style
                                .add_modifier
                                .contains(ratatui::style::Modifier::BOLD))
                            .map(|span| span.content.as_ref())
                            .collect::<String>()
                            .contains("Authored bold")),
                "authored message Markdown remains styled"
            );
            assert!(
                session.outgoing[0]
                    .ticket
                    .text
                    .ends_with("**Authored bold** #tmt:worker")
            );
        }
        app.view.as_mut().unwrap().replies.push(json!({
            "recipientId":MEMBER,"requestId":"literal-name","submittedAtMs":SENT+43_000,
            "response":"Exact reply body stays literal source evidence"
        }));
        crate::status::with_now_ms(SENT + 43_000, || board_frame(&app, 148, 51));
        let state = app.conversation.borrow();
        let session = state.sessions.values().next().unwrap();
        let mapped = &session.rendered.as_ref().unwrap().mapped;
        assert!(
            mapped
                .lines
                .iter()
                .any(|line| line.to_string().starts_with(&format!(" {NAME} → "))),
            "reply author names stay literal"
        );
        assert_eq!(mapped.mentions.iter().map(Vec::len).sum::<usize>(), 1);
        assert_eq!(
            session.outgoing[0].reply.as_ref().unwrap().body,
            "Exact reply body stays literal source evidence"
        );
        assert_eq!(
            fake.arguments().iter().filter(|arg| *arg == "talk").count(),
            1
        );
    });
}

#[test]
fn a_settled_send_stays_in_order_when_the_same_agent_asks_a_new_question() {
    let fake = Fake::new(json!({"requestId":"r1", "status":"queued"}), false);
    let mut app = fixture();
    let completed = send(&mut app, &fake, "first message");
    finish(&mut app, completed);
    let identity = app
        .conversation
        .borrow()
        .sessions
        .keys()
        .next()
        .cloned()
        .unwrap();
    let incoming = Reply {
        from: "auth-fix".into(),
        age: "now".into(),
        at_ms: None,
        body: "A new question while you were away".into(),
        key: Some(crate::board::home_leads::MessageKey {
            sender: identity.0.clone().unwrap(),
            lead: identity.1.clone(),
            squad: "product".into(),
            request: "incoming".into(),
            kind: crate::board::home_leads::Kind::Question,
        }),
        acquired: true,
        source: false,
    };
    app.conversation
        .borrow_mut()
        .sessions
        .get_mut(&identity)
        .unwrap()
        .update(Some(incoming));
    let state = app.conversation.borrow();
    let session = &state.sessions[&identity];
    assert_eq!(
        session.outgoing.len(),
        1,
        "the earlier send must stay below the new question's exchange"
    );
}

#[test]
fn the_item_cap_folds_only_the_oldest_send_and_keeps_order() {
    let mut session = Session::default();
    for sequence in 0..=MAX_ITEMS as u64 {
        session.outgoing.push_back(Entry {
            ticket: Ticket {
                key: ("ben-id".into(), "agent".into()),
                sequence,
                text: format!("message {sequence}"),
                squad: "product".into(),
            },
            started_ms: sequence,
            outcome: None,
            question: None,
            lead_in: None,
            reply: None,
            feedback: String::new(),
            feedback_clock: None,
            sending_shown_ms: None,
            sending_held: false,
        });
    }
    session.trim_outgoing();
    assert_eq!(session.outgoing.len(), MAX_ITEMS);
    assert_eq!(session.outgoing.front().unwrap().ticket.sequence, 1);
    assert_eq!(
        session.outgoing.back().unwrap().ticket.sequence,
        MAX_ITEMS as u64
    );
}

#[test]
fn sends_and_later_questions_alternate_in_arrival_order() {
    let fake = Fake::new(json!({"requestId":"r1", "status":"queued"}), false);
    let mut app = fixture();
    let completed = send(&mut app, &fake, "first send");
    finish(&mut app, completed);
    let identity = app
        .conversation
        .borrow()
        .sessions
        .keys()
        .next()
        .cloned()
        .unwrap();
    let question = |request: &str, body: &str| Reply {
        from: "auth-fix".into(),
        age: "now".into(),
        at_ms: None,
        body: body.into(),
        key: Some(crate::board::home_leads::MessageKey {
            sender: identity.0.clone().unwrap(),
            lead: identity.1.clone(),
            squad: "product".into(),
            request: request.into(),
            kind: crate::board::home_leads::Kind::Question,
        }),
        acquired: true,
        source: false,
    };
    let feed = |app: &App, reply: Reply| {
        app.conversation
            .borrow_mut()
            .sessions
            .get_mut(&identity)
            .unwrap()
            .update(Some(reply));
    };
    feed(&app, question("q2", "second question"));
    // The same question refreshed with a longer body updates in place.
    feed(&app, question("q2", "second question, revised"));
    // A newer question replaces only the unanswered tail, not the sends.
    feed(&app, question("q3", "third question"));
    let text = app.conversation.borrow().sessions[&identity].text.clone();
    assert_eq!(text.matches("You · now").count(), 1, "{text:?}");
    assert!(!text.contains("second question"), "{text:?}");
    assert!(
        text.ends_with("auth-fix · now\n\nthird question"),
        "{text:?}"
    );
    // Answering the tail keeps it above the new send, below the first one.
    if app.input.is_some() {
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
    let completed = send(&mut app, &fake, "second send");
    finish(&mut app, completed);
    let text = app.conversation.borrow().sessions[&identity].text.clone();
    let order: Vec<_> = ["first send", "third question", "second send"]
        .iter()
        .map(|part| {
            text.find(part)
                .unwrap_or_else(|| panic!("{part}: {text:?}"))
        })
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text:?}");
    let screen = draw(&app);
    let painted: Vec<_> = ["first send", "third question", "second send"]
        .iter()
        .map(|part| {
            screen
                .find(part)
                .unwrap_or_else(|| panic!("{part}: {screen:?}"))
        })
        .collect();
    assert!(
        painted.windows(2).all(|pair| pair[0] < pair[1]),
        "{screen:?}"
    );
}

#[test]
fn a_reply_still_being_read_never_flashes_a_loading_row_between_working_and_reply() {
    use crate::board::{activity::Phase, app::workspace::tests::MEMBER};
    const SENT: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(SENT, || {
        let mut app = crate::board::app::workspace::tests::board();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .reduced_motion = true;
        provider_observation(&mut app, Phase::Working, SENT);
        let fake = Fake::new(json!({"requestId":"slow","status":"sent"}), false);
        let completed = send(&mut app, &fake, "one explicit message");
        finish(&mut app, completed);
        let _no_acquisition = crate::runner::forbid_commands();
        let at = |app: &App, ms: u64| crate::status::with_now_ms(SENT + ms, || draw(app));
        assert!(at(&app, 2_000).contains("⠹ worker is working · 0:02"));
        // The final's metadata lands first and the provider goes idle, but its
        // body takes about 900 ms to read.
        app.view.as_mut().unwrap().replies.push(json!({
            "recipientId":MEMBER,"requestId":"slow","submittedAtMs":SENT + 3_000
        }));
        provider_observation(&mut app, Phase::Idle, SENT + 3_000);
        for ms in [3_100, 3_500, 3_950] {
            let screen = at(&app, ms);
            assert!(screen.contains("worker is working"), "{ms}: {screen}");
            assert!(
                !screen.contains("reading reply")
                    && !screen.contains("finished without replying")
                    && !screen.contains("Loading"),
                "{ms}: {screen}"
            );
        }
        app.view.as_mut().unwrap().replies.last_mut().unwrap()["response"] =
            json!("Slow exact reply");
        let screen = at(&app, 4_000);
        assert!(screen.contains("Slow exact reply"), "{screen}");
        assert!(
            !screen.contains("is working") && !screen.contains("reading reply"),
            "{screen}"
        );
    });
}

#[test]
fn the_sending_row_is_nothing_for_400ms_then_visible_for_at_least_400ms() {
    use crate::board::{activity::Phase, app::workspace::tests::MEMBER};
    const SENT: u64 = 1_791_600_000_000;
    let at = |app: &App, ms: u64| crate::status::with_now_ms(SENT + ms, || draw(app));
    let fresh = |text: &str, fake: &Fake| {
        let mut app = crate::board::app::workspace::tests::board();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .reduced_motion = true;
        provider_observation(&mut app, Phase::Working, SENT);
        let completed = crate::status::with_now_ms(SENT, || send(&mut app, fake, text));
        (app, completed)
    };
    let _ = MEMBER;
    crate::status::with_now_ms(SENT, || {
        // A fast send, admitted at 200 ms, never paints `sending…`.
        let fake = Fake::new(json!({"requestId":"fast","status":"sent"}), false);
        let (mut app, completed) = fresh("fast message", &fake);
        for ms in [0, 100, 190] {
            let screen = at(&app, ms);
            assert!(!screen.contains("sending"), "{ms}: {screen}");
        }
        crate::status::with_now_ms(SENT + 200, || finish(&mut app, completed));
        for ms in [210, 250, 600] {
            let screen = at(&app, ms);
            assert!(!screen.contains("sending"), "{ms}: {screen}");
            assert!(screen.contains("worker is working"), "{ms}: {screen}");
        }
        // A slow send shows `sending…` from 400 ms and keeps it for 400 ms
        // after it was first painted, even if the send settles right away.
        let fake = Fake::new(json!({"requestId":"slow","status":"sent"}), false);
        let (mut app, completed) = fresh("slow message", &fake);
        assert!(!at(&app, 399).contains("sending"));
        assert!(at(&app, 450).contains("sending…"));
        crate::status::with_now_ms(SENT + 500, || finish(&mut app, completed));
        let held = at(&app, 600);
        assert!(
            held.contains("sending…") && !held.contains("worker is working"),
            "{held}"
        );
        let done = at(&app, 900);
        assert!(
            !done.contains("sending") && done.contains("worker is working"),
            "{done}"
        );
    });
}

#[test]
fn a_hooked_agent_that_starts_late_shows_a_neutral_sent_and_never_not_reported() {
    use crate::board::{activity::Phase, app::workspace::tests::MEMBER};
    const SENT: u64 = 1_791_600_000_000;
    let _ = MEMBER;
    crate::status::with_now_ms(SENT, || {
        let mut app = crate::board::app::workspace::tests::board();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .reduced_motion = true;
        // The agent reports activity: idle before this send.
        provider_observation(&mut app, Phase::Idle, SENT - 60_000);
        let fake = Fake::new(json!({"requestId":"late","status":"sent"}), false);
        let completed = send(&mut app, &fake, "wake up");
        finish(&mut app, completed);
        let _no_acquisition = crate::runner::forbid_commands();
        let at = |app: &App, ms: u64| crate::status::with_now_ms(SENT + ms, || draw(app));
        for ms in [100, 1_000, 8_000] {
            let screen = at(&app, ms);
            assert!(screen.contains("✓ sent"), "{ms}: {screen}");
            assert!(!screen.contains("not reported"), "{ms}: {screen}");
        }
        // Hooks answer a moment later: straight to working, no false line between.
        provider_observation(&mut app, Phase::Working, SENT + 1_200);
        let screen = at(&app, 1_300);
        assert!(screen.contains("worker is working"), "{screen}");
        assert!(!screen.contains("not reported"), "{screen}");
    });
}

#[test]
fn a_restored_send_is_dropped_once_a_live_entry_has_the_same_request() {
    let mut app = fixture();
    let fake = Fake::new(json!({"requestId":"req-live","status":"sent"}), false);
    let completed = send(&mut app, &fake, "Sent in this session");
    finish(&mut app, completed);
    let mut state = app.conversation.borrow_mut();
    let session = state.sessions.values_mut().next().unwrap();
    let sent = |request: &str| crate::board::row_detail::Sent {
        request: request.into(),
        at_ms: 1,
        text: "text".into(),
        truncated: false,
    };
    session.sent = vec![sent("req-live"), sent("req-old")];
    let restored: Vec<_> = session.restored_sends().map(|send| &send.request).collect();
    assert_eq!(restored, ["req-old"]);
}
