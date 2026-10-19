use super::*;
use crate::test_support::TestDirectory;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write as _,
    time::{Duration, SystemTime},
};

const ROOT: &str = "00000000-0000-4000-8000-000000000001";
const OTHER_SEAT: &str = "00000000-0000-4000-8000-0000000000aa";

fn id(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

fn real_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

struct Sessions {
    directory: TestDirectory,
}

impl Sessions {
    fn new() -> Self {
        let directory = TestDirectory::new();
        fs::create_dir_all(directory.path.join("sessions")).unwrap();
        Self { directory }
    }

    fn root(&self) -> PathBuf {
        self.directory.path.join("sessions").canonicalize().unwrap()
    }

    /// `time` is the creation time in the file name, e.g. `2026-10-19T09-00-00`.
    fn rollout(&self, time: &str, session: &str, parent: Option<&str>) -> PathBuf {
        let day = self
            .root()
            .join(&time[0..4])
            .join(&time[5..7])
            .join(&time[8..10]);
        fs::create_dir_all(&day).unwrap();
        let path = day.join(format!("rollout-{time}-{session}.jsonl"));
        // The header carries the instructions, as a real one does.
        let header = json!({"type":"session_meta","payload":{
            "id": session, "parent_thread_id": parent,
            "base_instructions": "x".repeat(20_000),
        }});
        fs::write(&path, format!("{header}\n")).unwrap();
        path
    }

    fn model(&self, path: &Path, model: &str) {
        append(
            path,
            &json!({"type":"turn_context","payload":{"model":model}}),
        );
    }

    fn usage(&self, path: &Path, input: u64, output: u64, cached: u64) {
        append(
            path,
            &json!({"type":"event_msg","payload":{"type":"token_count","info":{
                "total_token_usage":{"input_tokens":input,"output_tokens":output,
                    "cached_input_tokens":cached,"total_tokens":input+output}}}}),
        );
    }

    fn age(&self, path: &Path, ms: u64) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_millis(ms))
            .unwrap();
    }
}

fn append(path: &Path, value: &Value) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{value}").unwrap();
}

struct Seat<'a> {
    sessions: &'a Sessions,
    session: String,
    main: PathBuf,
    others: HashSet<String>,
    stored: Option<Stored>,
}

impl<'a> Seat<'a> {
    fn new(sessions: &'a Sessions, session: &str, main: PathBuf) -> Self {
        Self {
            sessions,
            session: session.to_owned(),
            main,
            others: HashSet::new(),
            stored: None,
        }
    }

    /// One tick, applied to the stored family as storage would.
    fn tick(&mut self, now: u64) -> Outcome {
        let outcome = advance(
            &Tick {
                root: &self.sessions.root(),
                main: &self.main,
                session: &self.session,
                others: &self.others,
                now,
                deadline: Instant::now() + Duration::from_secs(30),
                corrupt: false,
            },
            self.stored.clone(),
        )
        .unwrap();
        self.stored = Some(Stored::after(self.stored.take(), &outcome.write));
        outcome
    }

    fn followed(&self) -> Vec<&str> {
        self.stored
            .as_ref()
            .map(|stored| stored.members.iter().map(|m| m.child.as_str()).collect())
            .unwrap_or_default()
    }
}

fn counts(input: u64, output: u64, cached: u64) -> Counts {
    Counts {
        input,
        output,
        cached,
    }
}

const T0: &str = "2026-10-19T09-00-00";
const T1: &str = "2026-10-19T09-10-00";
const T2: &str = "2026-10-19T09-20-00";

fn root_rollout(sessions: &Sessions) -> PathBuf {
    let main = sessions.rollout(T0, ROOT, None);
    sessions.usage(&main, 1_000, 100, 500);
    main
}

#[test]
fn the_first_scan_baselines_what_exists_and_counts_what_is_added_after_it() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let before = sessions.rollout(T1, &id(1), Some(ROOT));
    sessions.usage(&before, 400, 40, 100);
    let mut seat = Seat::new(&sessions, ROOT, main);
    let now = real_now();
    let first = seat.tick(now);
    // Existing usage is not new usage.
    assert_eq!(first.contribution.counts, Counts::default());
    assert!(first.contribution.complete);
    assert_eq!(seat.followed(), [id(1).as_str()]);
    // The same rollout then moves: only the movement counts.
    sessions.usage(&before, 450, 45, 120);
    // A rollout created after the first scan counts from zero.
    let later = sessions.rollout(T2, &id(2), Some(ROOT));
    sessions.usage(&later, 30, 3, 10);
    let second = seat.tick(now + 5_000);
    assert_eq!(second.contribution.counts, counts(80, 8, 30));
    assert!(second.contribution.complete);
    let again = seat.tick(now + 10_000);
    assert_eq!(again.contribution.counts, counts(80, 8, 30));
    assert!(again.write.upserts.is_empty() && again.write.deletes.is_empty());
    assert_eq!(again.write.header, None);
}

#[test]
fn a_child_of_a_child_and_an_auto_review_count_and_a_foreign_family_does_not() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let mut seat = Seat::new(&sessions, ROOT, main);
    let now = real_now();
    seat.tick(now);
    let child = sessions.rollout(T1, &id(1), Some(ROOT));
    sessions.usage(&child, 100, 10, 0);
    // Same second, sorting before its parent.
    let review = sessions.rollout(T1, &id(0), Some(&id(1)));
    sessions.usage(&review, 20, 2, 0);
    let foreign_root = sessions.rollout(T1, &id(7), None);
    sessions.usage(&foreign_root, 5_000, 500, 0);
    let foreign_child = sessions.rollout(T2, &id(8), Some(&id(7)));
    sessions.usage(&foreign_child, 700, 70, 0);
    let outcome = seat.tick(now + 5_000);
    assert_eq!(outcome.contribution.counts, counts(120, 12, 0));
    let mut followed = seat.followed();
    followed.sort_unstable();
    assert_eq!(followed, [id(0).as_str(), id(1).as_str()]);
}

#[test]
fn a_descendant_that_is_another_seats_session_counts_only_toward_that_seat() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let now = real_now();
    let mut first = Seat::new(&sessions, ROOT, main);
    first.others.insert(OTHER_SEAT.to_owned());
    first.tick(now);
    // The other seat's session names the first as its parent, and has a child.
    let other = sessions.rollout(T1, OTHER_SEAT, Some(ROOT));
    sessions.usage(&other, 9_000, 900, 0);
    let other_child = sessions.rollout(T2, &id(5), Some(OTHER_SEAT));
    sessions.usage(&other_child, 60, 6, 0);
    let plain = sessions.rollout(T2, &id(6), Some(ROOT));
    sessions.usage(&plain, 11, 1, 0);
    let outcome = first.tick(now + 5_000);
    assert_eq!(outcome.contribution.counts, counts(11, 1, 0));
    assert_eq!(first.followed(), [id(6).as_str()]);
    // The other seat's own family holds its child, and not the first's.
    let mut second = Seat::new(&sessions, OTHER_SEAT, other);
    second.others.insert(ROOT.to_owned());
    second.tick(now);
    let again = sessions.rollout("2026-10-19T09-30-00", &id(9), Some(OTHER_SEAT));
    sessions.usage(&again, 4, 1, 0);
    let outcome = second.tick(now + 5_000);
    // The baseline was taken at the first scan, which already held the child.
    assert_eq!(outcome.contribution.counts, counts(4, 1, 0));
    assert_eq!(second.followed().len(), 2);
    assert!(second.followed().contains(&id(5).as_str()));
}

#[test]
fn a_quiet_child_is_folded_into_the_closed_sum_and_never_counted_again() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let mut seat = Seat::new(&sessions, ROOT, main);
    let now = real_now();
    seat.tick(now);
    let child = sessions.rollout(T1, &id(1), Some(ROOT));
    sessions.usage(&child, 70, 7, 0);
    let live = seat.tick(now + 5_000);
    assert_eq!(live.contribution.counts, counts(70, 7, 0));
    assert_eq!(seat.followed().len(), 1);
    // A day with no write later, it is folded away.
    let folded = seat.tick(now + 5_000 + QUIET_MS + 1);
    assert_eq!(folded.contribution.counts, counts(70, 7, 0));
    assert!(seat.followed().is_empty());
    assert_eq!(folded.write.deletes.len(), 1);
    assert_eq!(
        seat.stored.as_ref().unwrap().header.closed,
        counts(70, 7, 0)
    );
    // It wakes up: its file is past the scan and is not followed again.
    sessions.usage(&child, 90, 9, 0);
    let later = seat.tick(now + 10_000 + QUIET_MS);
    assert_eq!(later.contribution.counts, counts(70, 7, 0));
    assert!(seat.followed().is_empty());
}

#[test]
fn discovery_is_bounded_and_the_family_is_incomplete_until_it_has_caught_up() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let count = CLASSIFY_PER_TICK + 10;
    for n in 0..count {
        let time = format!("2026-10-19T10-{:02}-{:02}", n / 60, n % 60);
        let child = sessions.rollout(&time, &id(100 + n as u32), Some(ROOT));
        sessions.usage(&child, 10, 1, 0);
    }
    let mut seat = Seat::new(&sessions, ROOT, main);
    let now = real_now();
    let first = seat.tick(now);
    assert!(!first.contribution.complete);
    assert_eq!(seat.followed().len(), CLASSIFY_PER_TICK);
    assert!(seat.stored.as_ref().unwrap().header.warm);
    let second = seat.tick(now + 5_000);
    assert!(second.contribution.complete);
    assert_eq!(seat.followed().len(), count);
    assert!(!seat.stored.as_ref().unwrap().header.warm);
    // Everything was baselined: no usage from before counting began.
    assert_eq!(second.contribution.counts, Counts::default());
}

#[test]
fn a_rollout_nobody_wrote_to_lately_is_not_read_while_warming() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let old = sessions.rollout(T1, &id(1), Some(ROOT));
    sessions.usage(&old, 5, 1, 0);
    sessions.age(&old, QUIET_MS + 60_000);
    let fresh = sessions.rollout(T2, &id(2), Some(ROOT));
    sessions.usage(&fresh, 5, 1, 0);
    let mut seat = Seat::new(&sessions, ROOT, main);
    let outcome = seat.tick(real_now());
    assert!(outcome.contribution.complete);
    assert_eq!(seat.followed(), [id(2).as_str()]);
}

#[test]
fn a_header_still_being_written_waits_and_does_not_advance_the_scan() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let mut seat = Seat::new(&sessions, ROOT, main);
    let now = real_now();
    seat.tick(now);
    let child = sessions.rollout(T1, &id(1), Some(ROOT));
    let line = fs::read(&child).unwrap();
    fs::write(&child, b"").unwrap();
    let waiting = seat.tick(now + 5_000);
    assert!(!waiting.contribution.complete);
    assert!(seat.followed().is_empty());
    fs::write(&child, line).unwrap();
    sessions.usage(&child, 40, 4, 0);
    let found = seat.tick(now + 10_000);
    assert!(found.contribution.complete);
    // It came after the first scan, so it counts from zero.
    assert_eq!(found.contribution.counts, counts(40, 4, 0));
}

#[test]
fn another_session_or_an_unreadable_family_is_replaced_not_extended() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let mut seat = Seat::new(&sessions, ROOT, main.clone());
    let now = real_now();
    seat.tick(now);
    let child = sessions.rollout(T1, &id(1), Some(ROOT));
    sessions.usage(&child, 40, 4, 0);
    seat.tick(now + 5_000);
    let stored = seat.stored.clone().unwrap();
    assert_eq!(stored.members.len(), 1);
    let others = HashSet::new();
    let root = sessions.root();
    let tick = |corrupt: bool, session: &str, stored: Option<Stored>| {
        advance(
            &Tick {
                root: &root,
                main: &main,
                session,
                others: &others,
                now: now + 10_000,
                deadline: Instant::now() + Duration::from_secs(30),
                corrupt,
            },
            stored,
        )
    };
    // The main rollout is not another session's: nothing is read for it.
    assert!(tick(false, &id(77), Some(stored.clone())).is_none());
    let replaced = tick(true, ROOT, Some(stored.clone())).unwrap();
    assert!(replaced.write.reset);
    // The rollout that existed is baselined, not counted.
    assert_eq!(replaced.write.upserts.len(), 1);
    assert_eq!(replaced.contribution.counts, Counts::default());
    let missing = tick(true, ROOT, None).unwrap();
    assert!(missing.write.reset);
    // A family stored for another session is replaced as well.
    let mut foreign = stored;
    foreign.header.session = id(55);
    let swapped = tick(false, ROOT, Some(foreign)).unwrap();
    assert!(swapped.write.reset);
}

#[test]
fn the_models_of_the_started_sessions_are_a_union_that_a_fifth_model_drops() {
    let row = |model: &str, input: u64| ModelUsage {
        model_id: model.to_owned(),
        input_tokens: input,
        output_tokens: 1,
        cached_input_tokens: 0,
        cache_write_tokens: None,
    };
    let merged = merge_rows(
        Some(vec![row("a", 10), row("b", 5)]),
        Some(vec![row("b", 7), row("c", 1)]),
    )
    .unwrap();
    assert_eq!(
        merged
            .iter()
            .map(|r| (r.model_id.as_str(), r.input_tokens))
            .collect::<Vec<_>>(),
        [("a", 10), ("b", 12), ("c", 1)]
    );
    assert_eq!(merge_rows(None, None), None);
    assert_eq!(
        merge_rows(None, Some(vec![row("a", 1)])),
        Some(vec![row("a", 1)])
    );
    assert_eq!(
        merge_rows(Some(vec![row("a", 1)]), None),
        Some(vec![row("a", 1)])
    );
    let four = vec![row("a", 1), row("b", 1), row("c", 1), row("d", 1)];
    assert_eq!(
        merge_rows(Some(four.clone()), Some(vec![row("e", 1)])),
        None
    );
    assert!(merge_rows(Some(four), Some(vec![row("d", 1)])).is_some());

    // Two children that moved in one tick, each with a model of its own.
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let mut seat = Seat::new(&sessions, ROOT, main);
    let now = real_now();
    seat.tick(now);
    for (n, model) in [(1u32, "gpt-a"), (2, "gpt-b")] {
        let child = sessions.rollout(T1, &id(n), Some(ROOT));
        sessions.model(&child, model);
        sessions.usage(&child, 100 * u64::from(n), 10, 0);
    }
    let outcome = seat.tick(now + 5_000);
    assert_eq!(outcome.contribution.counts, counts(300, 20, 0));
    let rows = outcome.contribution.rows.unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (r.model_id.as_str(), r.input_tokens))
            .collect::<Vec<_>>(),
        [("gpt-a", 100), ("gpt-b", 200)]
    );
}

#[test]
fn the_seat_total_is_the_session_plus_its_family_and_a_read_without_the_family_carries_it() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let mut seat = Seat::new(&sessions, ROOT, main.clone());
    let now = real_now();
    // The first read of the seat sets the baseline: the session's own counters.
    let family = seat.tick(now);
    let state = codex_with(
        &sessions.root(),
        &main,
        None,
        Some(&family.contribution),
        now,
    )
    .unwrap();
    assert_eq!(state.value.counts(), counts(1_000, 100, 500));
    let child = sessions.rollout(T1, &id(1), Some(ROOT));
    sessions.usage(&child, 200, 20, 50);
    let family = seat.tick(now + 5_000);
    let state = codex_with(
        &sessions.root(),
        &main,
        Some(&state),
        Some(&family.contribution),
        now + 5_000,
    )
    .unwrap();
    assert_eq!(state.value.counts(), counts(1_200, 120, 550));
    assert_eq!(state.value.sequence, 2);
    assert!(state.value.complete && !state.value.gap);
    // A hook-driven read does not look at the family and must not lose it.
    sessions.usage(&main, 1_100, 110, 550);
    let carried = codex_with(&sessions.root(), &main, Some(&state), None, now + 6_000).unwrap();
    assert_eq!(carried.value.counts(), counts(1_300, 130, 600));
    assert_eq!(carried.value.sequence, 3);
    assert_eq!(carried.value.epoch, state.value.epoch);
    // Nothing moved: the same state, the same sequence.
    assert_eq!(
        codex_with(&sessions.root(), &main, Some(&carried), None, now + 7_000),
        Some(carried.clone())
    );
    // The family moves alone.
    sessions.usage(&child, 260, 26, 60);
    let family = seat.tick(now + 8_000);
    let moved = codex_with(
        &sessions.root(),
        &main,
        Some(&carried),
        Some(&family.contribution),
        now + 8_000,
    )
    .unwrap();
    assert_eq!(moved.value.counts(), counts(1_360, 136, 610));
    assert_eq!(moved.value.sequence, 4);
    assert_eq!(moved.value.epoch, state.value.epoch);
}

#[test]
fn a_family_that_lost_ground_starts_a_new_baseline_like_a_counter_that_went_back() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let mut seat = Seat::new(&sessions, ROOT, main.clone());
    let now = real_now();
    let first = seat.tick(now);
    let state = codex_with(
        &sessions.root(),
        &main,
        None,
        Some(&first.contribution),
        now,
    )
    .unwrap();
    let child = sessions.rollout(T1, &id(1), Some(ROOT));
    sessions.usage(&child, 200, 20, 50);
    let family = seat.tick(now + 5_000);
    let state = codex_with(
        &sessions.root(),
        &main,
        Some(&state),
        Some(&family.contribution),
        now + 5_000,
    )
    .unwrap();
    // The stored family was lost and rebuilt: the contribution restarts at zero.
    let rebuilt = Contribution {
        counts: Counts::default(),
        rows: None,
        complete: true,
    };
    let after = codex_with(
        &sessions.root(),
        &main,
        Some(&state),
        Some(&rebuilt),
        now + 6_000,
    )
    .unwrap();
    assert!(after.value.gap && !after.value.complete);
    assert_ne!(after.value.epoch, state.value.epoch);
    assert_eq!(after.value.counts(), counts(1_000, 100, 500));
}

#[test]
fn an_incomplete_family_marks_the_total_incomplete_and_the_mark_survives_a_hook_read() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let now = real_now();
    let incomplete = Contribution {
        counts: counts(10, 1, 0),
        rows: None,
        complete: false,
    };
    let state = codex_with(&sessions.root(), &main, None, Some(&incomplete), now).unwrap();
    assert!(!state.value.complete);
    assert_eq!(state.value.counts(), counts(1_010, 101, 500));
    sessions.usage(&main, 1_100, 110, 550);
    let carried = codex_with(&sessions.root(), &main, Some(&state), None, now + 1).unwrap();
    assert!(
        !carried.value.complete,
        "the hook read still lacks the family"
    );
    let complete = Contribution {
        complete: true,
        ..incomplete
    };
    let done = codex_with(
        &sessions.root(),
        &main,
        Some(&carried),
        Some(&complete),
        now + 2,
    )
    .unwrap();
    assert!(done.value.complete);
    assert_eq!(done.value.sequence, carried.value.sequence + 1);
}

#[test]
fn the_state_with_a_family_round_trips_and_a_total_below_the_family_is_refused() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let now = real_now();
    let contribution = Contribution {
        counts: counts(10, 1, 0),
        rows: None,
        complete: true,
    };
    let state = codex_with(&sessions.root(), &main, None, Some(&contribution), now).unwrap();
    let document = state.document();
    assert_eq!(document["family"]["input"], 10);
    assert!(document["family"].get("incomplete").is_none());
    assert_eq!(State::read(&document), Some(state.clone()));
    let mut broken = document.clone();
    broken["family"]["input"] = json!(5_000_000);
    assert_eq!(
        State::read(&broken),
        None,
        "the family cannot exceed the total"
    );
    // A seat with no family carries no marker at all.
    let plain = codex_with(&sessions.root(), &main, None, None, now).unwrap();
    assert!(plain.document().get("family").is_none());
}

#[test]
fn the_worst_case_state_with_a_family_still_fits_the_opaque_bound() {
    let sessions = Sessions::new();
    let main = root_rollout(&sessions);
    let now = real_now();
    let big = 9_000_000_000_000u64;
    let contribution = Contribution {
        counts: counts(big / 2, big / 10, big / 4),
        rows: None,
        complete: false,
    };
    let state = codex_with(&sessions.root(), &main, None, Some(&contribution), now).unwrap();
    let document = serde_json::to_string(&state.document()).unwrap();
    assert!(document.len() < 700, "{} bytes: {document}", document.len());
}
