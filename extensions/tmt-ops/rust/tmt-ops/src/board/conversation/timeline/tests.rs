//! Committed public observations through the real conversation frame and viewport provenance.
use super::*;
use crate::board::{
    app::{
        Effect,
        workspace::tests::{MEMBER, board},
    },
    recent_responses::{Observation, Page},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::{Value, json};
use std::sync::Arc;

const VIEWER: &str = "33333333-3333-4333-8333-333333333333";
const OTHER: &str = "44444444-4444-4444-8444-444444444444";
const NOW: u64 = 1_791_600_000_000;
const QUESTION: &str = "Ben asks about CI and savings";
const A: &str = "Answer A about CI and savings";
const DIGEST: &str = "Core asks about the next fix";
const B: &str = "Answer B about the next fix";

fn input(id: &str, sender: &str, name: &str, text: &str) -> Value {
    json!({"entryId":id,"inputBatchId":"batch-g","kind":"input","requestId":id,
        "sender":{"identityId":sender,"name":name},"delivery":"delivered","availability":"retained",
        "text":text,"truncated":false,"originalBytes":text.len()})
}
fn final_entry(id: &str, response: &str, actor: &str, request: &str, revision: u64) -> Value {
    json!({"entryId":id,"inputBatchId":"batch-g","kind":"final","responseId":response,"availability":"retained",
        "acknowledgements":[{"identityId":actor,"requestId":request,"revision":revision}],"acknowledgementsTruncated":false})
}
fn response(id: &str, text: &str, ended: u64) -> Value {
    json!({"responseId":id,"text":text,"provider":"claude","endedMs":ended,"truncated":false,"originalBytes":text.len()})
}
fn incident() -> Value {
    json!({"identityId":MEMBER,"entries":[input("person",VIEWER,"Ben",QUESTION),
        final_entry("stop-z","rsp_A",VIEWER,"person",7),input("digest",OTHER,"core-1",DIGEST),
        final_entry("stop-a","rsp_B",OTHER,"digest",8)],
        "responses":[response("rsp_A",A,NOW-1000),response("rsp_B",B,NOW-5000)],
        "coverage":{"olderDropped":false},"capture":{"enabled":true}})
}
fn draw(app: &App, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 50)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn own_send(app: &mut App) {
    app.perform(&crate::action::Action::parse("talk").unwrap());
    app.paste(QUESTION);
    let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) else {
        panic!("send")
    };
    let ticket = app.outgoing_ticket(&request).unwrap();
    app.finished_send(outgoing::Completed {
        ticket: Some(ticket),
        outcome: Ok(crate::send::Accepted {
            request: "person".into(),
            disposition: crate::send::Disposition::Sent,
            passthrough: true,
            message_state_label: Some("✓ delivered".into()),
            can_send_now: false,
            working: false,
            fallback: None,
            warning: None,
        }),
        what: "Message".into(),
        to: "tmt-infra-lead".into(),
    });
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
}
fn setup(document: Value, live: bool) -> App {
    let mut app = board();
    let view = app.view.as_mut().unwrap();
    view.look = Look::default();
    view.me = Some("Ben".into());
    let workspace = &mut view.home.as_mut().unwrap().workspace;
    workspace.squad_looks.clear();
    workspace
        .rows
        .get_mut(app.workspace.selected.as_ref().unwrap())
        .unwrap()["name"] = json!("tmt-infra-lead");
    workspace.roster.members.get_mut(MEMBER).unwrap().name = "tmt-infra-lead".into();
    app.references.agents.insert(
        OTHER.into(),
        crate::board::inventory::Agent {
            name: "core-1".into(),
            canonical: "core-1".into(),
            ..Default::default()
        },
    );
    app.focus = 1;
    draw(&app, 160);
    if live {
        own_send(&mut app);
        draw(&app, 160);
    }
    let selected = app.conversation_finals().unwrap();
    app.apply_sent(crate::board::home_leads::SentKey {sender:selected.sender,agent:selected.agent},Ok(vec![json!({
        "requestId":"person","preparedAtMs":NOW-10000,"text":QUESTION,"truncated":false,"mode":"passthrough",
        "messageState":{"state":"delivered","reason":{"text":"delivered"}},
        "passthrough":{"state":"answered","boundAtMs":NOW-9000,"settledAtMs":NOW-900,
            "answer":{"responseId":"rsp_A","text":A,"truncated":false,"provider":"claude","endedMs":NOW-1000}}
    })]));
    app.conversation.borrow_mut().refresh_pane_output();
    let read = app.pane_output_plan().unwrap();
    let timeline = model::parse(MEMBER, &document).unwrap();
    app.apply_answer_read(
        &read,
        Observation::Observed(Arc::new(Page {
            coverage: timeline.coverage.clone(),
            capture: timeline.capture,
            timeline: Some(timeline),
            ..Default::default()
        })),
    );
    app
}

#[test]
fn upward_timeline_scroll_requests_the_opaque_older_page() {
    crate::status::with_now_ms(NOW, || {
        let mut document = incident();
        document["nextBefore"] = json!("opaque-older-page");
        let app = setup(document, false);
        draw(&app, 160);
        assert!(app.pane_output_plan().is_none(), "no eager older read");

        app.scroll_pane(
            crate::config::Pane::Replies,
            crate::board::scroll::Step::Top,
        );

        let read = app
            .pane_output_plan()
            .expect("Home requests the older timeline page");
        assert_eq!(
            read.before,
            Some(crate::board::recent_responses::Cursor::Timeline(
                "opaque-older-page".into()
            ))
        );
    });
}

#[test]
fn home_page_up_and_wheel_request_older_timeline_without_changing_the_recipient() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};

    crate::status::with_now_ms(NOW, || {
        for key in [Some(KeyCode::Home), Some(KeyCode::PageUp), None] {
            let mut document = incident();
            document["nextBefore"] = json!("opaque-older-page");
            let mut app = setup(document, false);
            app.focus = usize::from(key.is_some());
            draw(&app, 160);
            assert!(app.pane_output_plan().is_none(), "no eager older read");
            let selected = app.workspace.selected.clone();
            let focus = app.focused_pane();

            let effect = if let Some(key) = key {
                assert_eq!(focus, Some(crate::config::Pane::Replies));
                app.key(KeyEvent::new(key, KeyModifiers::NONE))
            } else {
                assert_eq!(focus, Some(crate::config::Pane::Rows));
                let pane = app.conversation.borrow().pane.unwrap();
                app.mouse(
                    MouseEvent {
                        kind: MouseEventKind::ScrollUp,
                        column: pane.x + 1,
                        row: pane.y + 1,
                        modifiers: KeyModifiers::NONE,
                    },
                    std::time::Instant::now(),
                )
            };
            assert_eq!(effect, Effect::None);
            assert_eq!(app.workspace.selected, selected);
            assert_eq!(app.focused_pane(), focus);
            assert_eq!(
                app.pane_output_plan()
                    .expect("input requests the older page")
                    .before,
                Some(crate::board::recent_responses::Cursor::Timeline(
                    "opaque-older-page".into()
                ))
            );
        }
    });
}

#[test]
fn incident_live_and_restored_paint_each_input_and_final_once_in_committed_order() {
    crate::status::with_now_ms(NOW, || {
        for live in [false, true] {
            for width in [160, 80] {
                let app = setup(incident(), live);
                let screen = draw(&app, width);
                let order = [QUESTION, A, DIGEST, B].map(|text| {
                    assert_eq!(
                        screen.matches(text).count(),
                        1,
                        "live={live} width={width} {text}: {screen}"
                    );
                    screen.find(text).unwrap()
                });
                assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{screen}");
                assert!(screen.contains("from core-1"), "{screen}");
                assert!(
                    !screen.contains("turn output") && !screen.contains("pane output"),
                    "{screen}"
                );
                let state = app.conversation.borrow();
                let session = &state.sessions[state.active.as_ref().unwrap()];
                let rendered = session.rendered.as_ref().unwrap();
                let row = rendered
                    .mapped
                    .lines
                    .iter()
                    .position(|line| line.to_string().contains(B))
                    .unwrap();
                assert_eq!(
                    rendered.mapped.lines[row].style.fg,
                    app.look().role(Role::Muted).fg
                );
            }
        }
    });
}

#[test]
fn only_painted_reply_body_rows_expose_the_viewers_frozen_revision() {
    crate::status::with_now_ms(NOW, || {
        let app = setup(incident(), false);
        draw(&app, 160);
        let state = app.conversation.borrow();
        let session = &state.sessions[state.active.as_ref().unwrap()];
        let rendered = session.rendered.as_ref().unwrap();
        for input in [QUESTION, DIGEST] {
            let row = rendered
                .mapped
                .lines
                .iter()
                .position(|line| line.to_string().contains(input))
                .unwrap();
            assert!(
                session
                    .painted_answers(&app, MEMBER, row..row + 1)
                    .is_empty()
            );
        }
        let row = rendered
            .mapped
            .lines
            .iter()
            .position(|line| line.to_string().contains(A))
            .unwrap();
        let keys = session.painted_answers(&app, MEMBER, row..row + 1);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].actor, VIEWER);
        assert_eq!(keys[0].request, "person");
        assert_eq!(keys[0].revision, Some(7));
        assert_eq!(keys[0].at_ms, None);
        let row = rendered
            .mapped
            .lines
            .iter()
            .position(|line| line.to_string().contains(B))
            .unwrap();
        assert!(
            session
                .painted_answers(&app, MEMBER, row..row + 1)
                .is_empty(),
            "another sender is not the viewer"
        );
    });
}

#[test]
fn a_pruned_final_uses_its_request_copy_by_response_identity() {
    crate::status::with_now_ms(NOW, || {
        let mut value = incident();
        value["entries"][1]["availability"] = json!("unavailable");
        value["responses"].as_array_mut().unwrap().remove(0);
        let app = setup(value, false);
        let screen = draw(&app, 160);
        assert_eq!(screen.matches(A).count(), 1, "{screen}");
        assert!(
            screen.find(A).unwrap() < screen.find(DIGEST).unwrap(),
            "{screen}"
        );
    });
}

#[test]
fn a_truncated_key_list_does_not_prove_the_viewer_is_absent() {
    let mut value = incident();
    value["entries"][1]["acknowledgements"] =
        json!([{"identityId":OTHER,"requestId":"other","revision":2}]);
    value["entries"][1]["acknowledgementsTruncated"] = json!(true);
    let page = model::parse(MEMBER, &value).unwrap();
    let inventory = crate::board::inventory::Inventory::default();
    assert_eq!(
        attribution(
            &page.entries[1],
            &page.entries[..1],
            Some(VIEWER),
            &inventory
        ),
        None
    );
    assert_eq!(
        attribution(&page.entries[1], &[], Some(VIEWER), &inventory),
        None
    );
    let mut complete = page.entries[1].clone();
    complete.acknowledgements_truncated = false;
    assert_eq!(
        attribution(&complete, &page.entries[..1], Some(VIEWER), &inventory),
        Some("from unknown sender".into())
    );
}

#[test]
fn a_slow_receipt_keeps_one_local_input_without_holding_other_inputs_or_replies() {
    crate::status::with_now_ms(NOW, || {
        let mut app = setup(incident(), false);
        draw(&app, 160);
        app.perform(&crate::action::Action::parse("talk").unwrap());
        let slow = "The slow local message";
        app.paste(slow);
        let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("send")
        };
        let ticket = app.outgoing_ticket(&request).unwrap();
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        draw(&app, 160);
        let mut document = incident();
        document["entries"].as_array_mut().unwrap().extend([
            input("slow-request", VIEWER, "Ben", slow),
            input(
                "other-new",
                OTHER,
                "core-1",
                "Another input during the slow send",
            ),
            final_entry("new-final", "rsp_new", VIEWER, "slow-request", 9),
        ]);
        document["responses"].as_array_mut().unwrap().push(response(
            "rsp_new",
            "A reply during the slow send",
            NOW,
        ));
        app.conversation.borrow_mut().refresh_pane_output();
        let read = app.pane_output_plan().unwrap();
        let timeline = model::parse(MEMBER, &document).unwrap();
        app.apply_answer_read(
            &read,
            Observation::Observed(Arc::new(Page {
                timeline: Some(timeline),
                ..Default::default()
            })),
        );
        let screen = draw(&app, 160);
        assert_eq!(screen.matches(slow).count(), 1, "{screen}");
        assert!(
            screen.contains("Another input during the slow send"),
            "{screen}"
        );
        assert!(screen.contains("A reply during the slow send"), "{screen}");
        assert!(screen.contains(QUESTION), "{screen}");
        assert!(
            screen.find(slow).unwrap() < screen.find("A reply during the slow send").unwrap(),
            "{screen}"
        );
        // A refused receipt releases the fence too: the committed input is
        // authoritative, while the failed local send remains a distinct draft.
        app.finished_send(outgoing::Completed {
            ticket: Some(ticket),
            outcome: Err(crate::core::SquadError::new("TIMEOUT", "Send timed out.")),
            what: "Message".into(),
            to: "tmt-infra-lead".into(),
        });
        draw(&app, 160);
        let state = app.conversation.borrow();
        let session = &state.sessions[state.active.as_ref().unwrap()];
        assert!(
            session
                .rendered
                .as_ref()
                .unwrap()
                .timeline_inputs
                .iter()
                .any(|id| id == "slow-request")
        );
    });
}

const SLOW: &str = "The slow local message";

/// Draws `incident()`, starts a send whose receipt never comes, admits `document` and
/// returns the screen.
fn admitted_during_a_slow_send(document: Value) -> String {
    let mut screen = String::new();
    crate::status::with_now_ms(NOW, || {
        let mut app = setup(incident(), false);
        draw(&app, 160);
        app.perform(&crate::action::Action::parse("talk").unwrap());
        app.paste(SLOW);
        let Effect::Act(_) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) else {
            panic!("send")
        };
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        draw(&app, 160);
        app.conversation.borrow_mut().refresh_pane_output();
        let read = app.pane_output_plan().unwrap();
        let timeline = model::parse(MEMBER, &document).unwrap();
        app.apply_answer_read(
            &read,
            Observation::Observed(Arc::new(Page {
                timeline: Some(timeline),
                ..Default::default()
            })),
        );
        screen = draw(&app, 160);
    });
    screen
}

#[test]
fn an_older_page_during_a_slow_send_keeps_its_own_inputs_painted_once() {
    const OLDER: &str = "An older own message";
    const BETWEEN: &str = "An own input between a painted input and a painted reply";
    let mut older = incident();
    older["entries"]
        .as_array_mut()
        .unwrap()
        .insert(0, input("older-own", VIEWER, "Ben", OLDER));
    let screen = admitted_during_a_slow_send(older);
    assert_eq!(screen.matches(OLDER).count(), 1, "{screen}");
    assert_eq!(screen.matches(SLOW).count(), 1, "{screen}");
    assert_eq!(screen.matches(QUESTION).count(), 1, "{screen}");
    assert!(
        screen.find(OLDER).unwrap() < screen.find(QUESTION).unwrap()
            && screen.find(QUESTION).unwrap() < screen.find(SLOW).unwrap(),
        "{screen}"
    );

    let mut between = incident();
    between["entries"]
        .as_array_mut()
        .unwrap()
        .insert(1, input("between-own", VIEWER, "Ben", BETWEEN));
    let screen = admitted_during_a_slow_send(between);
    assert_eq!(screen.matches(BETWEEN).count(), 1, "{screen}");
    assert_eq!(screen.matches(SLOW).count(), 1, "{screen}");
}

#[test]
fn unlinked_retained_history_is_not_erased_when_the_timeline_is_admitted() {
    crate::status::with_now_ms(NOW, || {
        let mut app = setup(incident(), false);
        draw(&app, 160);
        let selected = app.conversation_finals().unwrap();
        app.apply_sent(crate::board::home_leads::SentKey { sender: selected.sender, agent: selected.agent }, Ok(vec![json!({
            "requestId":"old-unlinked","preparedAtMs":NOW-20000,
            "text":"An older retained question","truncated":false,"mode":"passthrough",
            "passthrough":{"state":"answered","boundAtMs":NOW-19000,"settledAtMs":NOW-18000,
                "answer":{"responseId":"rsp_old","text":"An older retained answer","truncated":false,
                    "provider":"claude","endedMs":NOW-18500}}
        })]));
        let screen = draw(&app, 160);
        for text in [
            "An older retained question",
            "An older retained answer",
            QUESTION,
            A,
            DIGEST,
            B,
        ] {
            assert_eq!(screen.matches(text).count(), 1, "{text}: {screen}");
        }
        assert!(
            screen.find("An older retained answer").unwrap() < screen.find(QUESTION).unwrap(),
            "{screen}"
        );
    });
}

/// The same admitted incident observations used by the regression, through the
/// real painter in HOME's one decoded oracle.
pub(in crate::board) const SCENES: &[&str] = &[
    "timeline-incident-live",
    "timeline-incident-restored",
    "timeline-pruned",
    "timeline-uncertain",
    "timeline-failure",
    "timeline-input-unavailable",
    "timeline-mixed",
    "timeline-page-start",
];
pub(in crate::board) fn capture_case(scene: &str, look: Look) -> App {
    let mut document = incident();
    match scene {
        "timeline-incident-live" | "timeline-incident-restored" => {}
        "timeline-pruned" => {
            document["entries"][1]["availability"] = json!("unavailable");
            document["responses"].as_array_mut().unwrap().remove(0);
        }
        "timeline-uncertain" => document["entries"][2]["delivery"] = json!("uncertain"),
        "timeline-failure" => document["entries"].as_array_mut().unwrap().push(json!({
            "entryId":"failure","inputBatchId":"batch-g","kind":"failure",
            "failure":{"code":"budget_timeout"}})),
        "timeline-input-unavailable" => {
            let entry = &mut document["entries"][2];
            entry["availability"] = json!("unavailable");
            for field in ["text", "truncated", "originalBytes"] {
                entry.as_object_mut().unwrap().remove(field);
            }
            entry["sender"] = json!({});
        }
        "timeline-mixed" => {
            document["entries"].as_array_mut().unwrap().remove(1);
            document["entries"][2]["acknowledgements"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                "identityId":VIEWER,"requestId":"person","revision":7}));
        }
        "timeline-page-start" => {
            document["entries"].as_array_mut().unwrap().remove(0);
            document["entries"][0]["acknowledgements"] = json!([]);
        }
        _ => panic!("unknown timeline scene {scene}"),
    }
    let mut app = setup(document, scene == "timeline-incident-live");
    app.view.as_mut().unwrap().look = look;
    if scene == "timeline-page-start" {
        for session in app.conversation.borrow_mut().sessions.values_mut() {
            session.sent.clear();
            session.stale = true;
        }
    }
    app
}

#[test]
fn every_known_entry_age_follows_its_label_at_the_right_edge() {
    crate::status::with_now_ms(NOW, || {
        for width in [160, 80] {
            for live in [false, true] {
                let app = setup(incident(), live);
                let screen = draw(&app, width);
                let headers: Vec<_> = screen.lines().filter(|line| line.contains('→')).collect();
                assert!(headers[0].trim_end().ends_with("10s ago"), "{screen}");
                assert!(
                    headers[1].trim_end().ends_with("in turn · 1s ago"),
                    "{screen}"
                );
                assert!(
                    !headers[2].contains("ago"),
                    "no published input clock: {screen}"
                );
                assert!(
                    headers[3]
                        .trim_end()
                        .ends_with("in turn · from core-1 · 5s ago"),
                    "{screen}"
                );
            }
            for scene in ["chat-answered", "chat-distinct-identical"] {
                let app = crate::board::conversation::outgoing::tests::captures::fixture(
                    scene,
                    Look::default(),
                    NOW,
                );
                let screen = draw(&app, width);
                let headers: Vec<_> = screen.lines().filter(|line| line.contains('→')).collect();
                assert!(
                    headers[0].trim_end().ends_with("42s ago"),
                    "{scene}: {screen}"
                );
                assert!(
                    headers[1..]
                        .iter()
                        .all(|line| line.trim_end().ends_with("in turn · 1s ago")),
                    "{scene}: {screen}"
                );
            }
        }
    });
}

#[test]
fn known_entry_ages_advance_from_the_clock_without_another_read() {
    let app = crate::status::with_now_ms(NOW, || setup(incident(), false));
    crate::status::with_now_ms(NOW, || {
        let screen = draw(&app, 160);
        assert!(screen.contains("in turn · 1s ago"));
    });
    crate::status::with_now_ms(NOW + 1000, || {
        let screen = draw(&app, 160);
        assert!(screen.contains("in turn · 2s ago"), "{screen}");
        assert!(
            screen.contains("in turn · from core-1 · 6s ago"),
            "{screen}"
        );
        assert_eq!(screen.matches(A).count(), 1);
        assert_eq!(screen.matches(B).count(), 1);
    });
}

#[test]
fn a_folded_header_uses_the_first_known_input_clock() {
    crate::status::with_now_ms(NOW, || {
        let mut document = incident();
        document["entries"].as_array_mut().unwrap().insert(
            0,
            input(
                "older-no-clock",
                VIEWER,
                "Ben",
                "Older folded text without a public clock",
            ),
        );
        let app = setup(document, false);
        let screen = draw(&app, 160);
        let headers: Vec<_> = screen.lines().filter(|line| line.contains('→')).collect();
        assert_eq!(headers.len(), 4, "{screen}");
        assert!(headers[0].trim_end().ends_with("10s ago"), "{screen}");
        assert!(screen.contains("Older folded text without a public clock"));
        assert!(screen.contains(QUESTION));
    });
}
