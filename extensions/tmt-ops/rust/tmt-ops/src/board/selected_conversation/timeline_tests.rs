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
            dir.join("capabilities.json"),
            json!({"operations":["responses.timeline"]}).to_string(),
        )
        .unwrap();
        let recent = json!({"identityId":AGENT,"items":[],"coverage":{"olderDropped":false},"capture":{"enabled":true}});
        let input = json!({"requestId":"input-inline","kind":"request","sender":{"identityId":OWNER},"recipientId":AGENT,
            "prompt":{"status":"retained","message":"exact input"}});
        let reply = json!({"requestId":"reply-inline","kind":"request","sender":{"identityId":OWNER},"recipientId":AGENT,
            "final":{"status":"retained","response":"exact reply"}});
        let fake = dir.join("tmt");
        crate::test_support::write_ready_executable(
            &fake,
            &format!(
                "#!/bin/sh\nif [ \"$1\" != api ]; then printf '%s\\n' '{{}}'; exit 0; fi\nrequest=$(cat)\nprintf '%s\\n' \"$request\" >> '{}/calls'\ncase \"$request\" in\n*capabilities*) cat '{}/capabilities.json';;\n*responses.timeline*) cat '{}/page.json';;\n*responses.recent*) printf '%s\\n' '{}';;\n*input-inline*) printf '%s\\n' '{}';;\n*reply-inline*) printf '%s\\n' '{}';;\n*requests.show*) printf '%s\\n' '{}';;\n*requests.list*) printf '%s\\n' '{{\"items\":[],\"nextBefore\":null}}';;\n*) printf '%s\\n' '{{}}';;\nesac\n",
                dir.display(),
                dir.display(),
                dir.display(),
                recent,
                input,
                reply,
                question
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
    assert!(
        !calls
            .iter()
            .any(|call| call["operation"] == "requests.list")
    );
    assert_eq!(projection.bodies.len(), 1);
    assert_eq!(
        projection.bodies[0].1.as_ref().unwrap(),
        "exact independent question\r\n"
    );
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
    for message in &mut plan.messages {
        if message.key.kind == Kind::Question {
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
        3
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
