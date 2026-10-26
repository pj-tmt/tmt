//! Timeline acquisition owns covered bodies; only independent incoming questions need a show.
use super::*;
use serde_json::json;
use std::path::PathBuf;

const OWNER: &str = "550e8400-e29b-41d4-a716-446655440001";
const AGENT: &str = "550e8400-e29b-41d4-a716-446655440000";

struct Fixture {
    dir: PathBuf,
    core: Core,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ops-timeline-first-{}",
            crate::id::new_v4().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let page = json!({"identityId":AGENT,"entries":[
            {"entryId":"input","kind":"input","requestId":"input-inline",
             "sender":{"identityId":OWNER,"name":"viewer"},"delivery":"delivered",
             "availability":"retained","text":"exact input","truncated":false,"originalBytes":11},
            {"entryId":"reply","kind":"reply","requestId":"reply-inline",
             "availability":"retained","text":"exact reply","truncated":false,"originalBytes":11,
             "acknowledgements":[{"identityId":OWNER,"requestId":"reply-inline","revision":3}],
             "acknowledgementsTruncated":false}
        ],"responses":[],"coverage":{"olderDropped":false},"capture":{"enabled":true}});
        let question = json!({"requestId":"question-independent","kind":"request",
            "sender":{"identityId":AGENT},"recipientId":OWNER,
            "prompt":{"status":"retained","message":"exact independent question\r\n"}});
        std::fs::write(dir.join("page.json"), page.to_string()).unwrap();
        std::fs::write(
            dir.join("finals.json"),
            json!({"items":[],"nextBefore":null}).to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join("capabilities.json"),
            json!({"operations":["responses.timeline"]}).to_string(),
        )
        .unwrap();
        let recent = json!({"identityId":AGENT,"items":[],"coverage":{"olderDropped":false},"capture":{"enabled":true}});
        let input = json!({"requestId":"input-inline","kind":"request","sender":{"identityId":OWNER},"recipientId":AGENT,
            "prompt":{"status":"retained","message":"exact input"}});
        let reply = json!({"requestId":"reply-inline","kind":"request","sender":{"identityId":OWNER},"recipientId":AGENT,
            "final":{"status":"retained","response":"exact reply"}});
        let pending = json!({"items":[{"requestId":"pending-own","kind":"request",
            "sender":{"identityId":OWNER},"recipientId":AGENT,"preparedAtMs":1,
            "preview":"pending own preview","messageState":{"state":"batched","reason":{"text":"Core queued text"}}}],"nextBefore":null});
        let pending_detail = json!({"requestId":"pending-own","kind":"request",
            "sender":{"identityId":OWNER},"recipientId":AGENT,
            "prompt":{"status":"retained","message":"exact pending own"}});
        let fake = dir.join("tmt");
        crate::test_support::write_ready_executable(
            &fake,
            &format!(
                "#!/bin/sh\nif [ \"$1\" != api ]; then printf '%s\\n' '{{}}'; exit 0; fi\nrequest=$(cat)\nprintf '%s\\n' \"$request\" >> '{}/calls'\ncase \"$request\" in\n*capabilities*) cat '{}/capabilities.json';;\n*responses.timeline*) cat '{}/page.json';;\n*responses.recent*) printf '%s\\n' '{}';;\n*input-inline*) printf '%s\\n' '{}';;\n*reply-inline*) printf '%s\\n' '{}';;\n*pending-own*) printf '%s\\n' '{}';;\n*requests.show*) printf '%s\\n' '{}';;\n*results*) cat '{}/finals.json';;\n*requests.list*) printf '%s\\n' '{}';;\n*) printf '%s\\n' '{{}}';;\nesac\n",
                dir.display(),
                dir.display(),
                dir.display(),
                recent,
                input,
                reply,
                pending_detail,
                question,
                dir.display(),
                pending
            ),
        );
        Self {
            core: Core::at(fake),
            dir,
        }
    }
    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.join("calls"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}

#[test]
fn inline_timeline_bodies_are_acquired_before_legacy_history_and_only_questions_show() {
    let fixture = Fixture::new();
    let plan = plan();
    let projection = plan.read(
        &fixture.core,
        &mut SentTexts::default(),
        &Cancellation::default(),
    );
    let calls = fixture.calls();
    assert_eq!(
        calls[0]["operation"], "capabilities",
        "the timeline must precede legacy request hydration"
    );
    assert_eq!(calls[1]["operation"], "responses.timeline");
    let shows: Vec<_> = calls
        .iter()
        .filter(|call| call["operation"] == "requests.show")
        .collect();
    assert_eq!(shows.len(), 1, "inline bodies must not be fetched again");
    assert_eq!(shows[0]["input"]["requestId"], "question-independent");
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["operation"] == "requests.list")
            .count(),
        2,
        "one finals metadata list and one pending-send metadata list remain"
    );
    assert_eq!(projection.bodies.len(), 2);
    assert_eq!(
        projection.bodies[0].1.as_ref().unwrap(),
        "exact independent question\r\n"
    );
    assert_eq!(projection.bodies[1].0.request, "reply-inline");
    assert_eq!(projection.bodies[1].1.as_ref().unwrap(), "exact reply");
    let Some(recent_responses::Observation::Observed(page)) = projection.pane else {
        panic!("one coherent timeline publication")
    };
    assert_eq!(page.timeline.as_ref().unwrap().entries.len(), 2);
}

fn plan() -> Plan {
    Plan {
        key: FinalsKey {
            sender: OWNER.into(),
            agent: AGENT.into(),
        },
        squad: "product".into(),
        finals: true,
        sent: true,
        messages: [
            ("input-inline", Kind::Asked),
            ("reply-inline", Kind::Reply),
            ("question-independent", Kind::Question),
        ]
        .into_iter()
        .map(|(request, kind)| Message {
            key: MessageKey {
                sender: OWNER.into(),
                lead: AGENT.into(),
                squad: "product".into(),
                request: request.into(),
                kind,
            },
            at_ms: None,
            acquired: false,
        })
        .collect(),
        held_timeline: None,
        pane: Some(recent_responses::AnswerRead {
            key: (Some(OWNER.into()), AGENT.into()),
            before: None,
            held: None,
            rebuild: false,
        }),
    }
}

#[test]
fn unavailable_timeline_bodies_stay_unavailable_without_legacy_shows() {
    let fixture = Fixture::new();
    let path = fixture.dir.join("page.json");
    let mut page: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for entry in page["entries"].as_array_mut().unwrap() {
        entry["availability"] = json!("unavailable");
        for field in ["text", "truncated", "originalBytes"] {
            entry.as_object_mut().unwrap().remove(field);
        }
    }
    std::fs::write(path, page.to_string()).unwrap();
    let projection = plan().read(
        &fixture.core,
        &mut SentTexts::default(),
        &Cancellation::default(),
    );
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call["operation"] == "requests.show")
            .count(),
        1
    );
    assert_eq!(projection.bodies.len(), 1);
    let Some(recent_responses::Observation::Observed(page)) = projection.pane else {
        panic!("explicit unavailable entries")
    };
    assert!(
        matches!(&page.timeline.as_ref().unwrap().entries[0].kind,recent_responses::timeline::Kind::Input(input) if input.body.is_none())
    );
    assert!(matches!(
        &page.timeline.as_ref().unwrap().entries[1].kind,
        recent_responses::timeline::Kind::Reply { body: None, .. }
    ));
}

#[test]
fn held_timeline_survives_a_failed_refresh_without_rehydration_and_compares_by_pointer() {
    let fixture = Fixture::new();
    let mut plan = plan();
    let projection = plan.read(
        &fixture.core,
        &mut SentTexts::default(),
        &Cancellation::default(),
    );
    let Some(recent_responses::Observation::Observed(page)) = projection.pane else {
        panic!("initial page")
    };
    plan.held_timeline = Some(std::sync::Arc::clone(&page));
    plan.sent = false;
    plan.finals = false;
    for message in &mut plan.messages {
        if matches!(message.key.kind, Kind::Question | Kind::Reply) {
            message.acquired = true;
        }
    }
    plan.pane = None;
    let before = fixture.calls().len();
    let projection = plan.read(
        &fixture.core,
        &mut SentTexts::default(),
        &Cancellation::default(),
    );
    assert_eq!(
        fixture.calls().len(),
        before,
        "fresh retained page needs no acquisition"
    );
    assert!(projection.bodies.is_empty());
    assert_eq!(plan, plan.clone());
    let mut other = plan.clone();
    other.held_timeline = Some(std::sync::Arc::new((*page).clone()));
    assert_ne!(
        plan, other,
        "plan comparison never walks retained body text"
    );
    plan.pane = Some(recent_responses::AnswerRead {
        key: (Some(OWNER.into()), AGENT.into()),
        before: None,
        held: Some(page),
        rebuild: false,
    });
    std::fs::write(fixture.dir.join("page.json"), "{}").unwrap();
    let projection = plan.read(
        &fixture.core,
        &mut SentTexts::default(),
        &Cancellation::default(),
    );
    assert!(matches!(
        projection.pane,
        Some(recent_responses::Observation::Unavailable)
    ));
    assert!(projection.bodies.is_empty());
    assert_eq!(
        fixture.calls().len() - before,
        2,
        "only capability and failed timeline reads"
    );
}

#[test]
fn older_core_keeps_legacy_hydration_with_exact_source_bytes() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.dir.join("capabilities.json"),
        json!({"operations":["responses.recent"]}).to_string(),
    )
    .unwrap();
    let projection = plan().read(
        &fixture.core,
        &mut SentTexts::default(),
        &Cancellation::default(),
    );
    let calls = fixture.calls();
    assert!(
        calls
            .iter()
            .any(|call| call["operation"] == "responses.recent")
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["operation"] == "requests.list")
            .count(),
        2
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["operation"] == "requests.show")
            .count(),
        4
    );
    let bodies: std::collections::BTreeMap<_, _> = projection
        .bodies
        .into_iter()
        .map(|(key, body)| (key.request, body.unwrap()))
        .collect();
    assert_eq!(bodies["input-inline"], "exact input");
    assert_eq!(bodies["reply-inline"], "exact reply");
    assert_eq!(
        bodies["question-independent"],
        "exact independent question\r\n"
    );
}

#[test]
fn an_unhanded_own_send_keeps_its_core_state_and_preview_after_restart() {
    let fixture = Fixture::new();
    let projection = plan().read(
        &fixture.core,
        &mut SentTexts::default(),
        &Cancellation::default(),
    );
    let sent = projection.sent.unwrap().unwrap();
    assert_eq!(
        sent.len(),
        1,
        "the committed timeline excludes held sends, not their restoration metadata"
    );
    assert_eq!(sent[0]["requestId"], "pending-own");
    assert_eq!(sent[0]["text"], "pending own preview");
    assert_eq!(
        sent[0]["truncated"], true,
        "a list preview never becomes a full body"
    );
    assert_eq!(
        sent[0]["messageState"]["reason"]["text"],
        "Core queued text"
    );
    let calls = fixture.calls();
    assert!(
        !calls.iter().any(|call| call["operation"] == "requests.show"
            && call["input"]["requestId"] == "pending-own")
    );
}

#[test]
fn an_inline_reply_publication_finishes_loading_without_a_legacy_show() {
    use crate::board::{
        app::workspace::tests::{MEMBER, board},
        conversation::Refresh,
    };
    use ratatui::{Terminal, backend::TestBackend};
    let fixture = Fixture::new();
    let mut app = board();
    let owner = app.view.as_ref().unwrap().me_id.clone().unwrap();
    app.home_leads.test_results(
        &owner,
        json!({"items":[{
        "requestId":"reply-inline","kind":"request","sender":{"identityId":owner},
        "recipientId":MEMBER,"final":{"status":"retained","submittedAtMs":100},
        "responsePreview":"preview","recipientAcknowledged":true
    }],"nextBefore":null}),
    );
    let draw = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    draw(&app);
    let plan = app
        .conversation_read()
        .expect("initial selected acquisition");
    assert!(
        plan.messages
            .iter()
            .any(|message| message.key.kind == Kind::Reply && !message.acquired)
    );
    let mut page: Value =
        serde_json::from_slice(&std::fs::read(fixture.dir.join("page.json")).unwrap()).unwrap();
    page["identityId"] = json!(MEMBER);
    page["entries"][0]["sender"]["identityId"] = json!(owner);
    page["entries"][1]["acknowledgements"][0]["identityId"] = json!(owner);
    std::fs::write(fixture.dir.join("page.json"), page.to_string()).unwrap();
    let projection = plan.read(
        &fixture.core,
        &mut SentTexts::default(),
        &Cancellation::default(),
    );
    assert_eq!(projection.bodies.len(), 1);
    assert_eq!(projection.bodies[0].0.request, "reply-inline");
    assert_eq!(
        projection.bodies[0].1.as_ref().unwrap(),
        "exact reply",
        "source hydration reuses the acquired inline body"
    );
    app.selected_request = crate::board::refresh::SelectedRequest::Ready(Some(
        crate::board::refresh::SelectedRead::Conversation(plan.clone()),
    ));
    app.apply_conversation(plan, projection);
    let target = app.conversation_target().unwrap();
    assert_eq!(
        app.conversation_refresh(&target),
        Refresh::Current,
        "the timeline publication settles its own bodies"
    );
    let screen = draw(&app);
    assert!(screen.contains("exact reply"), "{screen}");
    assert!(!screen.contains("Loading conversation"), "{screen}");
    assert!(
        app.conversation_read().is_none(),
        "covered legacy bodies never schedule another read"
    );
    assert!(
        !fixture
            .calls()
            .iter()
            .any(|call| call["operation"] == "requests.show")
    );
}

#[test]
fn timeline_read_keeps_off_home_reply_age_and_exact_source_without_a_show() {
    assert_off_home_reply_metadata(false);
}

#[test]
fn common_pool_timeline_reply_keeps_age_and_exact_source_without_a_show() {
    assert_off_home_reply_metadata(true);
}

fn assert_off_home_reply_metadata(pool: bool) {
    use crate::board::app::workspace::tests::{MEMBER, board};
    use crate::board::workspace::OpsTarget;
    use ratatui::{Terminal, backend::TestBackend};
    let fixture = Fixture::new();
    let mut app = board();
    let previous = app.workspace.selected.clone().unwrap();
    let selected = if pool {
        OpsTarget::Member {
            room_id: None,
            identity_id: MEMBER.into(),
        }
    } else {
        previous.clone()
    };
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut row = snapshot.rows.remove(&previous).unwrap();
    if pool {
        snapshot.roster.squads[0].members.clear();
        row.as_object_mut().unwrap().remove("squad");
    } else {
        row["squad"] = json!("product");
    }
    snapshot.rows.insert(selected.clone(), row);
    app.workspace.pool_expanded = pool;
    app.workspace.selected = Some(selected);
    app.overview_rows.borrow_mut().clear();
    app.workspace_reconcile();
    let owner = app.view.as_ref().unwrap().me_id.clone().unwrap();
    let at = crate::status::now_ms().saturating_sub(42_000);
    let target = app.conversation_target().unwrap();
    assert!(
        app.detail(&Target::Row(target.clone()))
            .unwrap()
            .finals
            .is_empty(),
        "this reply is absent from both bounded home pages"
    );
    let page = json!({"identityId":MEMBER,"entries":[{
        "entryId":"older-reply","kind":"reply","requestId":"reply-off-home",
        "availability":"retained","text":"exact older reply body","truncated":false,
        "originalBytes":22,"acknowledgements":[{"identityId":owner,
        "requestId":"reply-off-home","revision":7}],"acknowledgementsTruncated":false
    }],"responses":[],"coverage":{"olderDropped":false},"capture":{"enabled":true}});
    let mut page = page;
    page["entries"].as_array_mut().unwrap().push(json!({
        "entryId":"unlisted-reply","kind":"reply","requestId":"reply-unlisted",
        "availability":"retained","text":"unlisted inline reply","truncated":false,
        "originalBytes":21,"acknowledgements":[],"acknowledgementsTruncated":false
    }));
    std::fs::write(fixture.dir.join("page.json"), page.to_string()).unwrap();
    std::fs::write(
        fixture.dir.join("finals.json"),
        json!({"items":[{
        "requestId":"reply-off-home","kind":"request","sender":{"identityId":owner},
        "recipientId":MEMBER,"final":{"status":"retained","submittedAtMs":at,"response":"WRONG list body"}
    }],"nextBefore":null})
        .to_string(),
    )
    .unwrap();
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.perform(&crate::action::Action::parse("talk").unwrap());
    app.paste("the completed own request");
    let crate::board::app::Effect::Act(send) =
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("local send")
    };
    let ticket = app.outgoing_ticket(&send).unwrap();
    app.finished_send(crate::board::conversation::outgoing::Completed {
        ticket: Some(ticket),
        outcome: Ok(crate::send::Accepted {
            request: "reply-off-home".into(),
            disposition: crate::send::Disposition::Sent,
            passthrough: false,
            message_state_label: None,
            can_send_now: false,
            working: false,
            fallback: None,
            warning: None,
        }),
        what: "Message".into(),
        to: "worker".into(),
    });
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let mut admitted = Terminal::new(TestBackend::new(160, 50)).unwrap();
    admitted
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let plan = app.conversation_read().unwrap();
    let projection = plan.read(
        &fixture.core,
        &mut SentTexts::default(),
        &Cancellation::default(),
    );
    app.apply_conversation(plan, projection);
    let detail = app.detail(&Target::Row(target)).unwrap();
    let reply = detail
        .finals
        .iter()
        .find(|reply| {
            reply
                .key
                .as_ref()
                .is_some_and(|key| key.request == "reply-off-home")
        })
        .expect("the targeted finals metadata survives the timeline publication");
    assert_eq!(reply.at_ms, Some(at));
    assert!(
        reply.acquired && reply.source,
        "the timeline body retains its exact request source"
    );
    assert_eq!(reply.body, "exact older reply body");
    let key = reply.key.as_ref().unwrap();
    assert_eq!(key.sender, owner);
    assert_eq!(key.lead, MEMBER);
    assert_eq!(key.kind, Kind::Reply);
    assert_eq!(key.squad, if pool { "Common pool" } else { "product" });
    let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("exact older reply body"), "{screen}");
    assert!(!screen.contains("WRONG list body"), "{screen}");
    assert!(
        !screen.contains("✓ sent"),
        "the settled reply replaces legacy send feedback: {screen}"
    );
    assert!(screen.contains("unlisted inline reply"), "{screen}");
    let rows: Vec<String> = terminal
        .backend()
        .buffer()
        .content
        .chunks(160)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let body_row = rows
        .iter()
        .position(|row| row.contains("unlisted inline reply"))
        .unwrap();
    let header = rows[..body_row]
        .iter()
        .rev()
        .find(|row| row.contains("in turn"))
        .unwrap();
    assert!(
        !header.contains("ago"),
        "an unlisted reply has no fabricated clock: {header}"
    );
    assert!(!detail.finals.iter().any(|reply| {
        reply
            .key
            .as_ref()
            .is_some_and(|key| key.request == "reply-unlisted")
    }));

    assert!(
        screen.contains("in turn ·") && screen.contains("ago"),
        "{screen}"
    );
    let calls = fixture.calls();
    assert_eq!(
        calls
            .iter()
            .filter(
                |call| call["operation"] == "requests.list" && call["input"]["view"] == "results"
            )
            .count(),
        1,
        "one bounded metadata read"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call["operation"] == "requests.show"),
        "the inline reply must not cause a legacy body read"
    );
}
