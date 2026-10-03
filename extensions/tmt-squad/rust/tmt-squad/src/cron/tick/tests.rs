use super::*;
use crate::cron::{Jobs, Pause, Schedule, ScheduleInput};
use std::collections::BTreeMap;

fn jobs() -> Vec<Job> {
    let mut jobs = Jobs::default();
    jobs.insert(Job::new(
        "product".into(),
        "11111111-1111-4111-8111-111111111111".into(),
        "22222222-2222-4222-8222-222222222222".into(),
        "literal {time}\n!\0 message".into(),
        Schedule::parse(
            ScheduleInput::Every {
                duration: "10s",
                from: None,
            },
            "UTC",
            0,
        )
        .unwrap(),
    ))
    .unwrap();
    jobs.jobs().to_vec()
}

#[derive(Default)]
struct Receiver {
    calls: Vec<String>,
    accepted: BTreeMap<String, (String, String, String)>,
    lose_reply: bool,
}
impl Dispatch for Receiver {
    fn send(&mut self, job: &Job, id: &str) -> Result<(), Error> {
        self.calls.push(id.into());
        let intent = (
            job.room_id.clone(),
            job.owner_id.clone().unwrap(),
            job.message.clone(),
        );
        if let Some(original) = self.accepted.get(id) {
            if original != &intent {
                return Err(Error::new("DISPATCH_CONFLICT", "Changed intent."));
            }
        } else {
            self.accepted.insert(id.into(), intent);
        }
        if self.lose_reply {
            return Err(Error::new("SQUAD_CORE_UNAVAILABLE", "Unknown outcome."));
        }
        Ok(())
    }
}

#[test]
fn running_window_sends_every_due_slot_once_and_limits_continuous_gaps_to_five_minutes() {
    let jobs = jobs();
    let mut receiver = Receiver::default();
    let mut tick = Tick::new(1);
    assert_eq!(tick.running(35_000, &jobs, &mut receiver).sent, 3);
    assert_eq!(tick.running(35_000, &jobs, &mut receiver).sent, 0);
    assert_eq!(tick.running(45_000, &jobs, &mut receiver).sent, 1);
    assert_eq!(tick.running(1_000_000, &jobs, &mut receiver).sent, 30);
    assert_eq!(
        receiver.calls[0],
        operation_id(&jobs[0].room_id, "c1", 10_000)
    );
    assert_eq!(
        receiver.calls[4],
        operation_id(&jobs[0].room_id, "c1", 710_000)
    );
    assert_eq!(receiver.accepted.len(), 34);
    assert!(
        receiver
            .accepted
            .values()
            .all(|(_, _, text)| text == "literal {time}\n!\0 message")
    );
}

#[test]
fn startup_takeover_restart_and_rollback_rebaseline_without_catching_up() {
    let jobs = jobs();
    let mut receiver = Receiver::default();
    for start in [0, 60_000, 300_000] {
        let mut tick = Tick::new(start);
        assert_eq!(tick.running(start, &jobs, &mut receiver).sent, 0);
        assert_eq!(tick.running(start + 10_000, &jobs, &mut receiver).sent, 1);
        let before = receiver.calls.len();
        assert_eq!(tick.running(start, &jobs, &mut receiver).sent, 0);
        assert_eq!(receiver.calls.len(), before);
        assert_eq!(tick.running(start + 10_000, &jobs, &mut receiver).sent, 1);
    }
    // A clock restarted after a long absence has no remembered watermark.
    let mut restarted = Tick::new(10_000_000);
    assert_eq!(restarted.running(10_000_000, &jobs, &mut receiver).sent, 0);
}

#[test]
fn standalone_ticks_share_the_last_sixty_seconds_and_deduplicate_across_clocks() {
    let jobs = jobs();
    let mut receiver = Receiver::default();
    assert_eq!(Tick::standalone(100_000, &jobs, &mut receiver).sent, 6);
    assert_eq!(
        receiver.calls[0],
        operation_id(&jobs[0].room_id, "c1", 50_000)
    );
    let original = receiver.accepted.clone();
    assert_eq!(Tick::standalone(100_000, &jobs, &mut receiver).sent, 6);
    assert_eq!(receiver.accepted, original);
    assert_eq!(receiver.calls.len(), 12);
}

#[test]
fn paused_and_ownerless_jobs_do_not_dispatch_but_other_jobs_still_do() {
    let mut jobs = jobs();
    let mut paused = jobs[0].clone();
    paused.pause = Some(Pause {
        by: "lead".into(),
        at_ms: 0,
    });
    jobs.push(paused.clone());
    paused.owner_id = None;
    jobs.push(paused);
    let mut receiver = Receiver::default();
    assert_eq!(Tick::standalone(60_000, &jobs, &mut receiver).sent, 6);
    assert_eq!(receiver.calls.len(), 6);
}

#[test]
fn uncertain_acceptance_reuses_slot_ids_and_changed_intent_conflicts() {
    let mut jobs = jobs();
    let mut receiver = Receiver {
        lose_reply: true,
        ..Receiver::default()
    };
    let report = Tick::standalone(60_000, &jobs, &mut receiver);
    assert_eq!(report.sent, 0);
    assert_eq!(report.failures.len(), 6);
    let original = receiver.accepted.clone();
    receiver.lose_reply = false;
    assert_eq!(Tick::standalone(60_000, &jobs, &mut receiver).sent, 6);
    assert_eq!(receiver.accepted, original);
    jobs[0].revision += 1;
    jobs[0].message.push_str(" changed");
    let report = Tick::standalone(60_000, &jobs, &mut receiver);
    assert_eq!(report.sent, 0);
    assert!(
        report
            .failures
            .iter()
            .all(|error| error.code == "DISPATCH_CONFLICT")
    );
    assert_eq!(receiver.accepted, original);
}

#[test]
fn slot_ids_separate_rooms_jobs_and_times_and_are_canonical_uuids() {
    let id = operation_id("room", "c1", 42);
    assert_ne!(id, operation_id("other-room", "c1", 42));
    assert_ne!(id, operation_id("room", "c2", 42));
    assert_ne!(id, operation_id("room", "c1", 43));
    assert_ne!(operation_id("a", "bc", 42), operation_id("ab", "c", 42));
    assert_eq!(id.len(), 36);
    assert_eq!(&id[14..15], "8");
    assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    assert!(
        id.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c) || c == '-')
    );
}
