use super::*;
use crate::board::unread::publish::{Completed, Outcome};
use std::sync::atomic::{AtomicU64, Ordering};

const ACTOR: &str = "33333333-3333-4333-8333-333333333333";
const OTHER: &str = "44444444-4444-4444-8444-444444444444";
const MEMBER: &str = "55555555-5555-4555-8555-555555555555";

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ops-local-unread-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn answer(index: usize) -> Answer {
    Answer {
        request: format!("request-{index}"),
        member: MEMBER.into(),
        answered_at_ms: 8_000_000,
        kind: Some(BodyKind::Final),
        revision: 1,
        acknowledged: true,
    }
}
fn cached(index: usize) -> Answer {
    let mut answer = answer(index);
    answer.acknowledged = false;
    answer
}
fn flush(state: &State, actor: &str, root: &Path) {
    state.start();
    while let Some(task) = state.offer(Some(actor)) {
        let result = task.read_update(root);
        task.settle(result).unwrap();
    }
}

#[test]
fn manual_mark_metadata_round_trips_without_text_or_core_ack_fields() {
    let root = Scratch::new();
    let state = State::default();
    state.mark(ACTOR, &answer(1)).unwrap();
    flush(&state, ACTOR, &root.0);
    let path = root.0.join(format!("{ACTOR}.json"));
    let bytes = fs::read(path).unwrap();
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(document["marks"][0].as_object().unwrap().len(), 5);
    assert!(document["marks"][0].get("acknowledged").is_none());
    let reopened = State::default();
    flush(&reopened, ACTOR, &root.0);
    assert_eq!(reopened.answers(ACTOR, MEMBER), vec![cached(1)]);
    assert!(
        !reopened.contains(OTHER, &answer(1)),
        "viewer namespace is exact"
    );
}

#[test]
fn the_501st_mark_is_refused_without_evicting_any_existing_mark() {
    let state = State::default();
    for index in 0..MAX_MARKS {
        state.mark(ACTOR, &answer(index)).unwrap();
    }
    let error = state.mark(ACTOR, &answer(MAX_MARKS)).unwrap_err();
    assert!(error.contains("500"));
    assert_eq!(state.answers(ACTOR, MEMBER).len(), MAX_MARKS);
    assert!(!state.contains(ACTOR, &answer(MAX_MARKS)));
    state.remove(ACTOR, vec![answer(0)]).unwrap();
    state.mark(ACTOR, &answer(MAX_MARKS)).unwrap();
    assert_eq!(state.answers(ACTOR, MEMBER).len(), MAX_MARKS);
}

#[test]
fn concurrent_boards_apply_deltas_to_current_bytes_and_keep_both_marks() {
    let root = Scratch::new();
    let first = State::default();
    let second = State::default();
    flush(&first, ACTOR, &root.0);
    flush(&second, ACTOR, &root.0);
    first.mark(ACTOR, &answer(1)).unwrap();
    second.mark(ACTOR, &answer(2)).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| flush(&first, ACTOR, &root.0));
        scope.spawn(|| flush(&second, ACTOR, &root.0));
    });
    let reopened = State::default();
    flush(&reopened, ACTOR, &root.0);
    assert!(reopened.contains(ACTOR, &answer(1)));
    assert!(reopened.contains(ACTOR, &answer(2)));
    first.remove(ACTOR, vec![answer(1)]).unwrap();
    flush(&first, ACTOR, &root.0);
    assert!(!first.contains(ACTOR, &answer(1)));
    assert!(
        first.contains(ACTOR, &answer(2)),
        "a stale board never replaces the whole set"
    );
}

#[test]
fn a_failed_publication_rolls_back_the_tentative_mark_and_requires_explicit_retry() {
    let root = Scratch::new();
    let blocked = root.0.join("not-a-directory");
    fs::write(&blocked, b"preserve").unwrap();
    let state = State::default();
    state.mark(ACTOR, &answer(1)).unwrap();
    let task = state.offer(Some(ACTOR)).unwrap();
    let result = task.read_update(&blocked);
    let error = task.settle(result).unwrap_err();
    assert!(error.contains("Local unread update failed"));
    assert!(error.contains("Press r"));
    assert_eq!(
        state.finish().unwrap_err(),
        error,
        "shutdown retains the original cause"
    );
    assert!(!state.contains(ACTOR, &answer(1)));
    assert!(
        state.offer(Some(ACTOR)).is_none(),
        "no automatic retry loop"
    );
    assert_eq!(fs::read(&blocked).unwrap(), b"preserve");
    state.mark(ACTOR, &answer(1)).unwrap();
    flush(&state, ACTOR, &root.0);
    assert!(state.contains(ACTOR, &answer(1)));
}

#[test]
fn an_automatic_ack_never_clears_a_mark_but_explicit_read_and_not_found_do() {
    let state = State::default();
    let held = answer(1);
    state.mark(ACTOR, &held).unwrap();
    let completed = Completed {
        actor: ACTOR.into(),
        items: vec![(held.clone(), Outcome::Acknowledged)],
    };
    state.completed(&completed).unwrap();
    assert!(state.contains(ACTOR, &held));
    state.read_after_ack(ACTOR, &held);
    state
        .completed(&Completed {
            actor: OTHER.into(),
            items: completed.items.clone(),
        })
        .unwrap();
    assert!(state.contains(ACTOR, &held));
    state.completed(&completed).unwrap();
    assert!(!state.contains(ACTOR, &held));
    state.mark(ACTOR, &held).unwrap();
    state
        .completed(&Completed {
            actor: ACTOR.into(),
            items: vec![(held.clone(), Outcome::NotFound)],
        })
        .unwrap();
    assert!(!state.contains(ACTOR, &held));
}

#[test]
fn a_new_revision_survives_a_stale_explicit_read_completion() {
    let state = State::default();
    let held = answer(1);
    state.mark(ACTOR, &held).unwrap();
    state.read_after_ack(ACTOR, &held);
    let mut fresh = held.clone();
    fresh.revision = 2;
    fresh.acknowledged = false;
    state.observe(&Page::retained(ACTOR, [fresh.clone()]));
    state
        .completed(&Completed {
            actor: ACTOR.into(),
            items: vec![(held, Outcome::Acknowledged)],
        })
        .unwrap();
    assert!(state.contains(ACTOR, &fresh));
    assert_eq!(state.answers(ACTOR, MEMBER)[0].revision, 2);
}

#[test]
fn corrupt_or_foreign_cache_bytes_are_not_overwritten() {
    let root = Scratch::new();
    let path = root.0.join(format!("{ACTOR}.json"));
    let foreign = br#"{"version":1,"actor":"other","marks":[]}"#;
    fs::write(&path, foreign).unwrap();
    let state = State::default();
    state.mark(ACTOR, &answer(1)).unwrap();
    let task = state.offer(Some(ACTOR)).unwrap();
    let result = task.read_update(&root.0);
    assert!(task.settle(result).is_err());
    assert_eq!(fs::read(path).unwrap(), foreign);
}

#[test]
fn locked_disk_capacity_refusal_preserves_all_preexisting_marks() {
    let root = Scratch::new();
    let full = State::default();
    for index in 0..MAX_MARKS {
        full.mark(ACTOR, &answer(index)).unwrap();
    }
    flush(&full, ACTOR, &root.0);
    let path = root.0.join(format!("{ACTOR}.json"));
    let before = fs::read(&path).unwrap();
    let stale = State::default();
    stale.mark(ACTOR, &answer(MAX_MARKS)).unwrap();
    let task = stale.offer(Some(ACTOR)).unwrap();
    let result = task.read_update(&root.0);
    let error = task.settle(result).unwrap_err();
    assert!(error.contains("500"));
    assert_eq!(fs::read(path).unwrap(), before);
    assert!(!stale.contains(ACTOR, &answer(MAX_MARKS)));
}

#[test]
fn fresh_metadata_fills_an_unknown_kind_at_the_same_revision() {
    let root = Scratch::new();
    let state = State::default();
    let mut unknown = answer(1);
    unknown.kind = None;
    state.mark(ACTOR, &unknown).unwrap();
    let known = answer(1);
    state.observe(&Page::retained(ACTOR, [known.clone()]));
    assert_eq!(state.answers(ACTOR, MEMBER)[0].kind, Some(BodyKind::Final));
    flush(&state, ACTOR, &root.0);
    let reopened = State::default();
    flush(&reopened, ACTOR, &root.0);
    reopened.mark(ACTOR, &unknown).unwrap();
    flush(&reopened, ACTOR, &root.0);
    assert_eq!(
        reopened.answers(ACTOR, MEMBER),
        vec![cached(1)],
        "an older unknown-kind mark never replaces known metadata"
    );
}
