use super::*;
use crate::board::cronboard::{Cron, State};
use tmt_ops::cron::{Holder, Job, Schedule, ScheduleInput};

pub(in crate::board) const NOW: i64 = 1_791_124_200_000; // 2026-10-04T14:30:00Z

pub(in crate::board) fn view(owner: &str, message: &str, next: Option<i64>) -> JobView {
    let schedule = Schedule::parse(
        ScheduleInput::Every {
            duration: "30m",
            from: None,
        },
        "Asia/Tokyo",
        NOW,
    )
    .unwrap();
    JobView {
        job: Job::new(
            "tmt-core".into(),
            "room".into(),
            "owner-id".into(),
            message.into(),
            schedule,
        ),
        owner_name: Some(owner.into()),
        owner_missing: false,
        next_ms: next.into_iter().collect(),
        warnings: vec![],
    }
}

/// `count` jobs of one squad with the distinct c-ids (`c1`, `c2`, ...) the store
/// assigns; the plain `view` fixture reuses one id.
pub(in crate::board) fn views(count: usize, squad: &str, room: &str) -> Vec<JobView> {
    let mut store = tmt_ops::cron::Jobs::default();
    (1..=count)
        .map(|index| {
            let mut job = view(
                "someone",
                &format!("job number {index}"),
                Some(NOW + 3_600_000),
            );
            job.job.squad = squad.into();
            job.job.room_id = room.into();
            store.insert(job.job.clone()).unwrap();
            job.job = store.jobs().last().unwrap().clone();
            job
        })
        .collect()
}

pub(in crate::board) fn cron(jobs: Vec<JobView>, clock: ClockStatus) -> Cron {
    Cron {
        jobs,
        clock,
        actor: Ok(crate::cron_service::CronActor {
            id: "user-id".into(),
            name: "Ben".into(),
        }),
        place: None,
        read_ms: NOW,
    }
}

#[test]
fn running_clock_age_matches_command_wording_and_steps_aside_whole() {
    let status = ClockStatus::Running(Holder {
        pane: Some("%41".into()),
        pid: 7,
        since_ms: NOW,
        expires_ms: NOW + 5_000,
        build: None,
    });
    for (elapsed, age) in [
        (-1, "just now"),
        (0, "just now"),
        (6_000, "6s ago"),
        (60_000, "1m ago"),
        (3_600_000, "1h ago"),
        (86_400_000, "1d ago"),
    ] {
        let now = NOW + elapsed;
        let full = format!("clock: %41 · {age}");
        assert_eq!(clock(&status, now, false, ClockNote::default()).0, full);
        assert_eq!(
            clock(&status, now, true, ClockNote::default()).0,
            "clock: %41"
        );
    }
}

#[test]
fn instants_use_the_stored_zone_and_show_the_date_when_not_today() {
    let next = NOW + 3_600_000;
    assert_eq!(
        time(next, NOW, "Asia/Tokyo").as_deref(),
        Some("Mon 10-05 00:30")
    );
    assert_eq!(time(next, NOW, "UTC").as_deref(), Some("15:30"));
    assert_eq!(time(next, NOW, "no-such-zone"), None);
    assert_eq!(time(i64::MAX, NOW, "UTC"), None);
}

#[test]
fn job_next_time_uses_its_first_observed_slot_and_unknown_never_becomes_zero() {
    let mut active = view("same-name", "instruction", Some(NOW + 1_800_000));
    active.next_ms.push(NOW + 7_200_000);
    assert_eq!(next_time(&active, NOW).as_deref(), Some("Mon 10-05 00:00"));
    active.next_ms.clear();
    assert_eq!(next_time(&active, NOW), None);
    active.next_ms.push(i64::MAX);
    assert_eq!(next_time(&active, NOW), None);
}

#[test]
fn failed_refresh_keeps_observed_jobs_actor_and_clock_until_a_new_success() {
    let mut state = State::default();
    state.replace(Err("not reported".into()));
    assert!(state.cron.is_none());
    assert_eq!(state.failure.as_deref(), Some("not reported"));
    assert_eq!(state.reads, 0);
    let loaded = cron(vec![view("owner", "unchanged", None)], ClockStatus::Unknown);
    state.replace(Ok(loaded));
    state.replace(Err("storage unreachable".into()));
    let retained = state.cron.as_ref().unwrap();
    assert_eq!(retained.jobs.len(), 1);
    assert_eq!(retained.jobs[0].job.message, "unchanged");
    assert_eq!(retained.actor.as_ref().unwrap().id, "user-id");
    assert_eq!(retained.clock, ClockStatus::Unknown);
    assert_eq!(state.reads, 1);
    assert!(state.clock_note(None).checking);
    state.replace(Ok(cron(vec![], ClockStatus::NoClock)));
    assert!(state.cron.as_ref().unwrap().jobs.is_empty());
    assert_eq!(state.failure, None);
    assert_eq!(state.reads, 2);
    assert!(!state.clock_note(None).checking);
}

#[test]
fn current_clock_and_job_text_neutralize_controls_and_remain_single_lines() {
    let text = "  investigate\u{1b}[31m\u{202e}界 e\u{301}\nsecond line";
    let shown = first_line(text);
    assert!(shown.contains("investigate"));
    assert!(!shown.contains(['\u{1b}', '\u{202e}', '\n']));
    assert!(!shown.contains("second line"));
    let status = ClockStatus::Running(Holder {
        pane: Some("%4\u{1b}[2J1\nforged".into()),
        pid: 7,
        since_ms: NOW,
        expires_ms: NOW,
        build: None,
    });
    let shown = clock(&status, NOW, false, ClockNote::default()).0;
    assert!(!shown.contains(['\u{1b}', '\n']));
    assert!(!shown.contains("forged"));
}
