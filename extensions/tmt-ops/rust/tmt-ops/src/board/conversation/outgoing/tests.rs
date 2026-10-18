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
fn send(app: &mut App, fake: &Fake, text: &str) -> Completed {
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
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
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
        assert_eq!(
            arguments.last().unwrap(),
            "[product · auth-fix] exact outgoing text"
        );
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
            "[product · auth-fix] literal auth-fix · now",
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
                .position(|line| line == &format!("[product · auth-fix] message {index}"))
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

#[test]
fn delivery_and_acquired_agent_freshness_remain_independent_after_failed_reads() {
    use crate::board::workspace::OpsObservation;
    for source in ["roster", "squad", "membership", "read-error"] {
        let mut app = crate::board::app::workspace::tests::board();
        let fake = Fake::new(json!({"requestId":"req-freshness","status":"sent"}), false);
        let completed = send(&mut app, &fake, "one explicit message");
        finish(&mut app, completed);
        let _no_acquisition = crate::runner::forbid_commands();
        draw(&app);
        assert!(
            !app.conversation
                .borrow()
                .sessions
                .values()
                .next()
                .unwrap()
                .agent_mark
                .contains("stale")
        );
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
            _ => unreachable!(),
        }
        draw(&app);
        let state = app.conversation.borrow();
        let session = state.sessions.values().next().unwrap();
        // A failed roster read makes core activity stale itself; other
        // failures keep the activity and mark the read stale.
        let expected = if source == "roster" {
            "worker stale · working"
        } else {
            "worker working · stale"
        };
        assert!(
            session.agent_mark.contains(expected),
            "{source}: {}",
            session.agent_mark
        );
        assert!(
            session.text.contains("✓ delivered"),
            "{source}: delivery stays the typed acceptance fact"
        );
        assert_eq!(session.outgoing.len(), 1, "a failed read never resends");
    }
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
        app.shell.tab = OpsTab::Overview;
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
    assert_eq!(
        app.conversation.borrow().sessions[&identity].text,
        "New exact question"
    );
}

#[test]
fn header_and_status_show_core_activity_even_when_the_task_disagrees() {
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
        let state = app.conversation.borrow();
        let mark = &state.sessions.values().next().unwrap().agent_mark;
        assert!(mark.ends_with(&format!("worker {words}")), "status: {mark}");
        assert!(
            !mark.contains("✗"),
            "the task never picks the status glyph: {mark}"
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
