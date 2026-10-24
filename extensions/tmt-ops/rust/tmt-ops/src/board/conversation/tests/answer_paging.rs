use super::*;
use crate::board::recent_responses::{Coverage, Item, Observation, Page};
use std::sync::Arc;

pub(in crate::board) const SCENES: [&str; 6] = [
    "answer-older-page",
    "answer-truncated",
    "answer-dropped",
    "answer-capture-disabled",
    "answer-capture-disabled-empty",
    "answer-page-failed",
];

pub(in crate::board) fn capture_case(scene: &str, look: Look) -> App {
    assert!(SCENES.contains(&scene));
    let mut app = workspace_fixture();
    app.focus = 1;
    let view = app.view.as_mut().unwrap();
    view.look = look;
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    view.replies[0]["response"] = json!("The stored reply stays distinct from pane answers.");
    view.replies[0]["submittedAtMs"] = json!(crate::status::now_ms() - 120_000);
    draw(&app, 160, 36);
    let read = app.pane_output_plan().unwrap();
    let mut value = (*page(0, 2, Some(50))).clone();
    for (index, item) in value.items.iter_mut().enumerate() {
        item.ended_ms = crate::status::now_ms() - 60_000 + index as u64 * 10_000;
        item.text = format!("Retained answer {index}\nThe exact answer remains readable.");
        item.original_bytes = item.text.len() as u64;
    }
    match scene {
        "answer-truncated" => {
            value.items[1].truncated = true;
            value.items[1].original_bytes = 20_000;
        }
        "answer-dropped" => {
            value.next_before = None;
            value.coverage = Coverage {
                older_dropped: true,
                dropped_through_ms: Some(crate::status::now_ms() - 180_000),
            };
        }
        "answer-capture-disabled" | "answer-capture-disabled-empty" => {
            value.capture = Some(false);
            value.next_before = None;
            if scene.ends_with("-empty") {
                value.items.clear();
            }
        }
        _ => {}
    }
    app.apply_answer_read(&read, Observation::Observed(Arc::new(value)));
    draw(&app, 160, 36);
    app.scroll_pane(Pane::Replies, Step::Top);
    if scene == "answer-page-failed" {
        let older = app.pane_output_plan().unwrap();
        app.apply_answer_read(&older, Observation::Unavailable);
    }
    if matches!(scene, "answer-truncated" | "answer-page-failed") {
        app.scroll_pane(Pane::Replies, Step::Bottom);
    }
    app
}

#[test]
fn every_oracle_scene_shows_its_answer_state_at_all_capture_widths() {
    let _no_commands = crate::runner::forbid_commands();
    for (scene, width) in SCENES
        .into_iter()
        .flat_map(|scene| [160, 100, 80, 60].map(|width| (scene, width)))
    {
        let mut app = capture_case(scene, Look::default());
        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        let expected = match scene {
            "answer-truncated" => "answer truncated",
            "answer-dropped" => "older answers not kept",
            "answer-capture-disabled" | "answer-capture-disabled-empty" => {
                "answers are not captured"
            }
            "answer-page-failed" => "Older answers unavailable",
            _ => "Retained answer 0",
        };
        assert!(text.contains(expected), "{scene} at {width}: {text}");
    }
}

fn page(start: u64, end: u64, next: Option<u64>) -> Arc<Page> {
    Arc::new(Page {
        items: (start..end)
            .map(|index| {
                let text = format!("Answer {index}\nExact second line {index}");
                Item {
                    original_bytes: text.len() as u64,
                    text,
                    provider: "codex".into(),
                    ended_ms: 3_600_000,
                    truncated: false,
                    session: Some("session".into()),
                }
            })
            .collect(),
        coverage: Coverage::default(),
        next_before: next,
        capture: Some(true),
    })
}

#[test]
fn upward_page_demand_uses_the_same_lane_and_prepend_preserves_the_reading_anchor() {
    crate::status::with_now_ms(12_000_000, || {
        let app = workspace_fixture();
        draw(&app, 100, 24);
        let initial = app.pane_output_plan().unwrap();
        assert!(initial.before.is_none());
        app.apply_answer_read(&initial, Observation::Observed(page(50, 100, Some(50))));
        draw(&app, 100, 24);
        assert!(app.pane_output_plan().is_none(), "no eager older read");
        let start = {
            let state = app.conversation.borrow();
            state.sessions[state.active.as_ref().unwrap()]
                .rendered
                .as_ref()
                .unwrap()
                .pane_start
                .unwrap()
        };
        app.scrolls.restore([(Pane::Replies, start + 1)]);
        app.scroll_pane(Pane::Replies, Step::Lines(-1));
        let older = app
            .pane_output_plan()
            .expect("deliberate upward boundary crossing");
        assert_eq!(older.before, Some(50));
        let before = {
            let state = app.conversation.borrow();
            state.sessions[state.active.as_ref().unwrap()]
                .rendered
                .as_ref()
                .unwrap()
                .mapped
                .lines[start]
                .to_string()
        };
        app.apply_answer_read(&older, Observation::Observed(page(0, 100, None)));
        draw(&app, 100, 24);
        let state = app.conversation.borrow();
        let session = &state.sessions[state.active.as_ref().unwrap()];
        assert!(!session.follow);
        let rendered = session.rendered.as_ref().unwrap();
        assert_eq!(
            rendered.mapped.lines[app.scrolls.offset(Pane::Replies)].to_string(),
            before
        );
        assert!(
            rendered.mapped.lines[app.scrolls.offset(Pane::Replies) + 1]
                .to_string()
                .contains("Answer 50"),
            "identical name/age headers must keep the original answer beneath the reader"
        );
        assert!(
            rendered
                .mapped
                .lines
                .iter()
                .any(|line| line.to_string().contains("Answer 0"))
        );
        for (index, line) in rendered.mapped.lines.iter().enumerate() {
            if line.to_string().contains("Answer 0") {
                assert!(session.response_key_at(index).is_none());
            }
        }
        drop(state);
        assert!(app.pane_output_plan().is_none());
        // Re-delivering a consumed cursor cannot append the page a second time.
        app.apply_answer_read(&older, Observation::Observed(page(0, 100, None)));
        let state = app.conversation.borrow();
        let Observation::Observed(held) =
            &state.sessions[state.active.as_ref().unwrap()].pane_output
        else {
            panic!()
        };
        assert_eq!(held.len(), 100);
    });
}

#[test]
fn failed_older_page_keeps_answers_and_cursor_without_an_automatic_retry() {
    let app = workspace_fixture();
    draw(&app, 100, 24);
    let initial = app.pane_output_plan().unwrap();
    let held = page(50, 100, Some(50));
    app.apply_answer_read(&initial, Observation::Observed(Arc::clone(&held)));
    draw(&app, 100, 24);
    app.scroll_pane(Pane::Replies, Step::Top);
    let read = app.pane_output_plan().unwrap();
    app.apply_answer_read(&read, Observation::Unavailable);
    draw(&app, 100, 24);
    assert!(app.pane_output_plan().is_none());
    app.conversation.borrow_mut().refresh_pane_output();
    let latest = app.pane_output_plan().unwrap();
    app.apply_answer_read(&latest, Observation::Observed(Arc::clone(&held)));
    assert!(app.pane_output_plan().is_none());
    {
        let state = app.conversation.borrow();
        let session = &state.sessions[state.active.as_ref().unwrap()];
        let Observation::Observed(after) = &session.pane_output else {
            panic!()
        };
        assert!(Arc::ptr_eq(after, &held));
        assert!(
            session.pane_failure == Some(AnswerFailure::Older),
            "a successful latest read did not retry the failed older cursor"
        );
        assert!(
            session
                .rendered
                .as_ref()
                .unwrap()
                .mapped
                .lines
                .iter()
                .any(|line| line.to_string().contains("Older answers unavailable"))
        );
    }
    app.scroll_pane(Pane::Replies, Step::Top);
    assert_eq!(app.pane_output_plan().unwrap().before, Some(50));
}

#[test]
fn a_latest_refresh_that_exhausts_history_consumes_an_older_page_demand() {
    let app = workspace_fixture();
    draw(&app, 100, 24);
    let read = app.pane_output_plan().unwrap();
    app.apply_answer_read(&read, Observation::Observed(page(50, 100, Some(50))));
    draw(&app, 100, 24);
    app.scroll_pane(Pane::Replies, Step::Top);
    app.conversation.borrow_mut().refresh_pane_output();
    let latest = app.pane_output_plan().unwrap();
    assert_eq!(latest.before, None);
    app.apply_answer_read(&latest, Observation::Observed(page(70, 100, None)));
    assert!(
        app.pane_output_plan().is_none(),
        "a spent cursor must not start an automatic read loop"
    );
}

#[test]
fn replaced_cursor_chain_and_sender_reject_late_older_completions() {
    let mut app = workspace_fixture();
    draw(&app, 100, 24);
    let initial = app.pane_output_plan().unwrap();
    app.apply_answer_read(&initial, Observation::Observed(page(50, 100, Some(50))));
    draw(&app, 100, 24);
    app.scroll_pane(Pane::Replies, Step::Top);
    let older = app.pane_output_plan().unwrap();
    app.conversation.borrow_mut().refresh_pane_output();
    let latest = app.pane_output_plan().unwrap();
    let replacement = page(40, 100, Some(40));
    app.apply_answer_read(&latest, Observation::Observed(Arc::clone(&replacement)));
    app.apply_answer_read(&older, Observation::Observed(page(0, 100, None)));
    {
        let state = app.conversation.borrow();
        let Observation::Observed(held) = &state.sessions[&older.key].pane_output else {
            panic!()
        };
        assert!(Arc::ptr_eq(held, &replacement));
    }
    let current = app.pane_output_plan().unwrap();
    app.view.as_mut().unwrap().me_id = Some("different-person-uuid".into());
    app.apply_answer_read(&current, Observation::Observed(page(0, 100, None)));
    let state = app.conversation.borrow();
    let Observation::Observed(held) = &state.sessions[&older.key].pane_output else {
        panic!()
    };
    assert!(Arc::ptr_eq(held, &replacement));
}

#[test]
fn scrolling_a_live_screen_does_not_request_an_older_chat_page() {
    let mut app = workspace_fixture();
    draw(&app, 100, 24);
    let read = app.pane_output_plan().unwrap();
    app.apply_answer_read(&read, Observation::Observed(page(50, 100, Some(50))));
    draw(&app, 100, 24);
    app.toggle_screen();
    app.scroll_pane(Pane::Replies, Step::Top);
    {
        let state = app.conversation.borrow();
        assert!(!state.sessions[&read.key].older_demand);
    }
    app.toggle_screen();
    assert!(app.pane_output_plan().is_none());
}

#[test]
fn capture_and_coverage_captions_keep_data_and_never_create_request_authority() {
    let mut session = Session::default();
    session.transcript_context = transcript::Context {
        enabled: true,
        name: "worker".into(),
        exchanges: transcript::Exchanges::Observed,
        dropped_age: Some("2m ago".into()),
        ..Default::default()
    };
    let mut value = (*page(0, 1, None)).clone();
    value.capture = Some(false);
    value.coverage = Coverage {
        older_dropped: true,
        dropped_through_ms: Some(42),
    };
    value.items[0].truncated = true;
    session.pane_output = Observation::Observed(Arc::new(value.clone()));
    for width in [60, 100, 160] {
        session.prepare(width, Look::default(), &Default::default());
        let rendered = session.rendered.as_ref().unwrap();
        let lines = rendered
            .mapped
            .lines
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>();
        assert_eq!(lines[0], "older answers not kept · at or before 2m ago");
        let text = lines
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("answers are not captured for this agent: run tmt setup"));
        assert!(text.contains("answer truncated · full text in the agent's pane"));
        assert!(text.contains("Answer 0"));
        assert!(rendered.final_spans.is_empty());
        assert!(
            rendered
                .response_sources
                .as_ref()
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
    }
    value.capture = None;
    value.next_before = Some(1);
    session.pane_output = Observation::Observed(Arc::new(value));
    session.stale = true;
    session.prepare(100, Look::default(), &Default::default());
    let text = session
        .rendered
        .as_ref()
        .unwrap()
        .mapped
        .lines
        .iter()
        .map(Line::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("older answers not kept"));
    assert!(!text.contains("answers are not captured"));
}
