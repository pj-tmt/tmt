//! A passthrough's typed text is matched to the turn it started by the prompt the provider
//! reported, and the turn's end settles what it earned (#4336). The events arrive through the
//! runtime-observation transaction, so only an admitted event (one that changed the remembered
//! state) may bind or settle.
use super::{tests::fixture, *};
use crate::storage::ResponseCapture;
use std::time::Duration;
use tmt_core::{
    request::passthrough::{BIND_WINDOW_MS, BOUND_WINDOW_MS, CaptureState},
    responses::FinalText,
};

fn now() -> u64 {
    crate::request_runtime::wall_time_ms()
}

/// A passthrough typed into `agent`'s pane `ago` milliseconds ago, still waiting for its turn.
fn typed(storage: &Storage, agent: &str, request: &str, text: &str, ago: u64) {
    let delivered = now() - ago;
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO request_attempts (
                attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
                inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
                message_text,message_bytes,message_expires_at_ms
            ) VALUES (?1,?2,'inbox',?3,0,'sent',0,0,?4,?5,?6,?7,?8,?6)",
            rusqlite::params![
                format!("attempt-{request}"),
                request,
                agent,
                delivered as i64,
                (delivered + 3_600_000) as i64,
                (delivered + 604_800_000) as i64,
                text,
                text.len() as i64
            ],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO request_passthrough(request_id,delivered_at_ms,state) VALUES(?1,?2,'pending')",
            rusqlite::params![request, delivered as i64],
        )
        .unwrap();
}

/// One admitted main-session event: the remembered state moves, and `boundary` rides along.
fn event(
    storage: &mut Storage,
    path: &std::path::Path,
    id: &str,
    session: &str,
    boundary: Option<TurnBoundary<'_>>,
    response: Option<&str>,
    admitted: bool,
) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static MOVES: AtomicU64 = AtomicU64::new(0);
    let expected = Storage::context_by_identity(path, id, now())
        .unwrap()
        .unwrap();
    let mut preferences = expected.preferences.clone();
    if admitted {
        let remembered = preferences.remembered.as_mut().unwrap();
        remembered.provider_session =
            tmt_core::binding::session::ProviderSessionId::new(session).unwrap();
        remembered.state = Some(
            tmt_core::binding::session::DriverState::new(
                1,
                &format!(
                    r#"{{"model":"move-{}"}}"#,
                    MOVES.fetch_add(1, Ordering::SeqCst)
                ),
            )
            .unwrap(),
        );
    }
    storage
        .commit_runtime_observation(RuntimeObservation {
            expected: &expected,
            preferences: &preferences,
            remember_source: false,
            locator: None,
            sampled: false,
            consumption: None,
            now_ms: now(),
            deadline: Instant::now() + Duration::from_secs(2),
            response: response.map(|text| ResponseCapture {
                text: FinalText::new(text).unwrap(),
                retention_days: 1,
            }),
            boundary,
            family: None,
        })
        .unwrap()
}

fn capture(storage: &Storage, request: &str) -> tmt_core::request::passthrough::PassthroughCapture {
    super::super::requests::passthrough::capture(storage.connection().unwrap(), request)
        .unwrap()
        .unwrap()
}

const SESSION: &str = "fixture-session";

#[test]
fn a_turn_binds_by_prompt_text_and_its_final_answers_the_passthrough_once() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    typed(&storage, &id, "p1", "Ship it!", 1_000);
    // The provider reports what the pane received: `！` for `!`.
    let started = TurnBoundary::Started {
        turn: Some("t1"),
        prompt: "Ship it！",
    };
    assert!(event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(started),
        None,
        true
    ));
    let bound = capture(&storage, "p1");
    assert_eq!(bound.state, CaptureState::Bound);
    assert_eq!(bound.bound_turn.as_deref(), Some("t1"));
    assert_eq!(bound.bound_session.as_deref(), Some(SESSION));
    assert!(bound.answer.is_none());

    let ended = TurnBoundary::Ended { turn: Some("t1") };
    assert!(event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(ended),
        Some("Shipped."),
        true
    ));
    let answered = capture(&storage, "p1");
    assert_eq!(answered.state, CaptureState::Answered);
    let answer = answered.answer.clone().unwrap();
    assert_eq!(answer.text, "Shipped.");
    assert_eq!(answer.provider, "claude");
    assert!(!answer.truncated);
    assert_eq!(answered.settled_at_ms, Some(answer.ended_ms));

    // A repeated end, or another turn's final, never rewrites what was settled.
    let again = TurnBoundary::Ended { turn: Some("t1") };
    event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(again),
        Some("Something else."),
        true,
    );
    assert_eq!(capture(&storage, "p1"), answered);
    storage.close().unwrap();
}

#[test]
fn identical_pending_texts_bind_first_in_first_out() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    typed(&storage, &id, "first", "go", 2_000);
    typed(&storage, &id, "second", "go", 1_000);
    for (turn, bound, waiting, final_text) in [
        ("t1", "first", "second", "answer one"),
        ("t2", "second", "first", "answer two"),
    ] {
        let started = TurnBoundary::Started {
            turn: Some(turn),
            prompt: "go",
        };
        event(&mut storage, &path, &id, SESSION, Some(started), None, true);
        assert_eq!(
            capture(&storage, bound).state,
            CaptureState::Bound,
            "{turn}"
        );
        if turn == "t1" {
            assert_eq!(capture(&storage, waiting).state, CaptureState::Pending);
        }
        let ended = TurnBoundary::Ended { turn: Some(turn) };
        event(
            &mut storage,
            &path,
            &id,
            SESSION,
            Some(ended),
            Some(final_text),
            true,
        );
    }
    assert_eq!(
        capture(&storage, "first").answer.unwrap().text,
        "answer one"
    );
    assert_eq!(
        capture(&storage, "second").answer.unwrap().text,
        "answer two"
    );
    storage.close().unwrap();
}

#[test]
fn another_turn_neither_binds_nor_lends_its_final_and_a_late_prompt_cannot_bind() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    typed(&storage, &id, "queued", "do the thing", 1_000);
    // A busy agent works another prompt first: its turn and final are not the passthrough's.
    let other = TurnBoundary::Started {
        turn: Some("busy"),
        prompt: "an unrelated prompt",
    };
    event(&mut storage, &path, &id, SESSION, Some(other), None, true);
    let busy_end = TurnBoundary::Ended { turn: Some("busy") };
    event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(busy_end),
        Some("unrelated final"),
        true,
    );
    let waiting = capture(&storage, "queued");
    assert_eq!(
        waiting.state,
        CaptureState::Pending,
        "never guessed from the next final"
    );
    assert!(waiting.answer.is_none());
    // Its own turn starts later and binds.
    let own = TurnBoundary::Started {
        turn: Some("own"),
        prompt: "do the thing",
    };
    event(&mut storage, &path, &id, SESSION, Some(own), None, true);
    assert_eq!(
        capture(&storage, "queued").bound_turn.as_deref(),
        Some("own")
    );

    // Past the binding window the same prompt binds nothing, and the read says so.
    typed(&storage, &id, "late", "too late", BIND_WINDOW_MS + 1_000);
    let late = TurnBoundary::Started {
        turn: Some("t9"),
        prompt: "too late",
    };
    event(&mut storage, &path, &id, SESSION, Some(late), None, true);
    let stale = capture(&storage, "late");
    assert_eq!(
        stale.state,
        CaptureState::Pending,
        "storage holds it, unbound"
    );
    assert_eq!(stale.state_at(now()), CaptureState::NoAnswer);
    storage.close().unwrap();
}

#[test]
fn a_turn_that_ends_without_a_final_settles_without_an_answer() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    typed(&storage, &id, "p1", "think", 1_000);
    let started = TurnBoundary::Started {
        turn: None,
        prompt: "think",
    };
    event(&mut storage, &path, &id, SESSION, Some(started), None, true);
    // Claude gives no turn id: the session orders turns.
    assert_eq!(capture(&storage, "p1").bound_turn, None);
    let ended = TurnBoundary::Ended { turn: None };
    event(&mut storage, &path, &id, SESSION, Some(ended), None, true);
    let settled = capture(&storage, "p1");
    assert_eq!(settled.state, CaptureState::NoAnswer);
    assert!(settled.answer.is_none());
    assert!(settled.settled_at_ms.is_some());
    storage.close().unwrap();
}

#[test]
fn the_answer_is_capped_and_says_it_was_cut() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    typed(&storage, &id, "p1", "long please", 1_000);
    let started = TurnBoundary::Started {
        turn: Some("t"),
        prompt: "long please",
    };
    event(&mut storage, &path, &id, SESSION, Some(started), None, true);
    let ended = TurnBoundary::Ended { turn: Some("t") };
    event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(ended),
        Some(&"é".repeat(6_000)),
        true,
    );
    let answer = capture(&storage, "p1").answer.unwrap();
    assert!(answer.truncated);
    assert!(
        answer.text.len() <= 8_192 && answer.text.len() > 8_000,
        "{}",
        answer.text.len()
    );
    storage.close().unwrap();
}

#[test]
fn an_event_that_changed_nothing_binds_and_settles_nothing() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    typed(&storage, &id, "p1", "once", 1_000);
    let started = TurnBoundary::Started {
        turn: Some("t"),
        prompt: "once",
    };
    // A duplicate or stale provider event is not admitted: the remembered state does not move.
    event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(started),
        None,
        false,
    );
    assert_eq!(capture(&storage, "p1").state, CaptureState::Pending);
    event(&mut storage, &path, &id, SESSION, Some(started), None, true);
    assert_eq!(capture(&storage, "p1").state, CaptureState::Bound);
    let ended = TurnBoundary::Ended { turn: Some("t") };
    event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(ended),
        Some("unadmitted"),
        false,
    );
    assert_eq!(capture(&storage, "p1").state, CaptureState::Bound);
    storage.close().unwrap();
}

#[test]
fn a_capture_bound_to_an_earlier_session_is_released_when_the_agent_moves_on() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    typed(&storage, &id, "old", "first prompt", 2_000);
    typed(&storage, &id, "new", "second prompt", 1_000);
    let started = TurnBoundary::Started {
        turn: Some("t1"),
        prompt: "first prompt",
    };
    event(&mut storage, &path, &id, SESSION, Some(started), None, true);
    // The conversation was replaced before the first turn ended; its end will never be seen.
    let next = TurnBoundary::Started {
        turn: Some("t1"),
        prompt: "second prompt",
    };
    event(
        &mut storage,
        &path,
        &id,
        "session-2",
        Some(next),
        None,
        true,
    );
    assert_eq!(capture(&storage, "old").state, CaptureState::NoAnswer);
    assert_eq!(capture(&storage, "new").state, CaptureState::Bound);
    storage.close().unwrap();
}

fn started(
    storage: &mut Storage,
    path: &std::path::Path,
    id: &str,
    turn: Option<&str>,
    prompt: &str,
) {
    let boundary = TurnBoundary::Started { turn, prompt };
    event(storage, path, id, SESSION, Some(boundary), None, true);
}

fn move_bound_time(storage: &Storage, request: &str, ago: u64) {
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE request_passthrough SET bound_at_ms=?2 WHERE request_id=?1",
            rusqlite::params![request, (now() - ago) as i64],
        )
        .unwrap();
}

#[test]
fn a_later_turn_of_the_same_session_settles_a_bound_capture_without_an_answer() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    typed(&storage, &id, "named", "first job", 3_000);
    typed(&storage, &id, "unnamed", "second job", 2_000);
    typed(&storage, &id, "same", "third job", 1_000);

    // The provider names its turns: a different turn overtakes, the same turn does not.
    started(&mut storage, &path, &id, Some("t1"), "first job");
    assert_eq!(capture(&storage, "named").state, CaptureState::Bound);
    started(&mut storage, &path, &id, Some("t1"), "an unrelated prompt");
    assert_eq!(
        capture(&storage, "named").state,
        CaptureState::Bound,
        "the same turn reporting again has not ended it"
    );
    started(&mut storage, &path, &id, Some("t2"), "an unrelated prompt");
    let overtaken = capture(&storage, "named");
    assert_eq!(overtaken.state, CaptureState::NoAnswer);
    assert!(overtaken.settled_at_ms.is_some() && overtaken.answer.is_none());
    // The earlier turn's end, arriving late, gives it nothing.
    let ended = TurnBoundary::Ended { turn: Some("t1") };
    event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(ended),
        Some("Too late."),
        true,
    );
    assert_eq!(capture(&storage, "named"), overtaken);

    // Claude names no turn: any later turn of the session overtakes.
    started(&mut storage, &path, &id, None, "second job");
    assert_eq!(capture(&storage, "unnamed").state, CaptureState::Bound);
    started(&mut storage, &path, &id, None, "an unrelated prompt");
    assert_eq!(capture(&storage, "unnamed").state, CaptureState::NoAnswer);

    // The turn that binds a capture does not overtake it.
    started(&mut storage, &path, &id, None, "third job");
    assert_eq!(capture(&storage, "same").state, CaptureState::Bound);
    storage.close().unwrap();
}

#[test]
fn a_bound_capture_whose_turn_never_ends_settles_without_an_answer_after_the_bound_window() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    typed(&storage, &id, "slow", "long job", 2_000);
    typed(&storage, &id, "stuck", "lost job", 1_000);
    started(&mut storage, &path, &id, Some("slow-turn"), "long job");
    // A long turn inside the window still answers when it ends.
    move_bound_time(&storage, "slow", BOUND_WINDOW_MS - 60_000);
    let ended = TurnBoundary::Ended {
        turn: Some("slow-turn"),
    };
    event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(ended),
        Some("Done at last."),
        true,
    );
    assert_eq!(capture(&storage, "slow").state, CaptureState::Answered);

    // One past the window is not awaited any more, even if its end arrives afterwards.
    started(&mut storage, &path, &id, Some("lost-turn"), "lost job");
    move_bound_time(&storage, "stuck", BOUND_WINDOW_MS + 1_000);
    let ended = TurnBoundary::Ended {
        turn: Some("lost-turn"),
    };
    event(
        &mut storage,
        &path,
        &id,
        SESSION,
        Some(ended),
        Some("Never wanted."),
        true,
    );
    let stuck = capture(&storage, "stuck");
    assert_eq!(stuck.state, CaptureState::NoAnswer);
    assert!(stuck.answer.is_none());
    storage.close().unwrap();
}
