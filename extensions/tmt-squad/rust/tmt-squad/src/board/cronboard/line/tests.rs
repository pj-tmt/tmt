use super::*;
use tmt_squad::cron::{Holder, Job, Schedule, ScheduleInput};

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
        next_ms: next.into_iter().collect(),
        warnings: vec![],
    }
}

pub(in crate::board) fn cron(jobs: Vec<JobView>, clock: ClockStatus) -> Cron {
    Cron {
        jobs,
        clock,
        actor: Ok(crate::cron_service::CronActor {
            id: "user-id".into(),
            name: "Ben".into(),
        }),
        read_ms: NOW,
    }
}

fn running() -> ClockStatus {
    ClockStatus::Running(Holder {
        pane: Some("%41".into()),
        pid: 7,
        since_ms: NOW - 12 * 60_000,
        expires_ms: NOW + 5_000,
    })
}

fn state(cron: Cron) -> State {
    State {
        cron: Some(cron),
        failure: None,
    }
}

fn text(state: &State, width: u16) -> String {
    line(state, NOW, width, Look::default())
        .unwrap()
        .to_string()
}

#[test]
fn home_line_keeps_count_time_clock_and_key_at_supported_widths() {
    for width in [160, 100, 80] {
        for clock in [running(), ClockStatus::NoClock, ClockStatus::Unknown] {
            let jobs = vec![view("tmt-lead", "merge queue sweep", Some(NOW + 3_600_000))];
            let state = state(cron(jobs, clock));
            let rendered = line(&state, NOW, width, Look::default()).unwrap();
            let shown = rendered.to_string();
            assert!(rendered.width() <= usize::from(width), "{width}: {shown}");
            assert!(shown.starts_with("⑤ ⏱ 1 "), "{shown}");
            assert!(shown.contains("next Mon 10-05 00:30"), "{shown}");
            assert!(shown.ends_with("c list"), "{shown}");
        }
    }
}

#[test]
fn the_preview_and_owner_step_aside_before_the_clock_and_key() {
    let jobs = vec![view(
        "tmt-lead",
        "merge queue sweep and check every pending review for the leads",
        Some(NOW + 3_600_000),
    )];
    let state = state(cron(jobs, ClockStatus::NoClock));
    let wide = text(&state, 160);
    assert!(wide.contains("tmt-lead merge queue sweep"), "{wide}");
    assert!(wide.contains("no clock · tmt sq cron run"), "{wide}");
    let narrow = text(&state, 80);
    assert!(narrow.contains("no clock"), "{narrow}");
    assert!(narrow.ends_with("c list"), "{narrow}");
    assert!(!narrow.contains("merge queue sweep and check"), "{narrow}");
}

#[test]
fn no_jobs_invents_no_next_slot_and_paused_jobs_have_none() {
    let none = text(&state(cron(vec![], ClockStatus::Unknown)), 100);
    assert!(none.starts_with("⑤ ⏱ 0 cron jobs"), "{none}");
    assert!(!none.contains("next"), "{none}");
    assert!(none.contains("clock: unknown"), "{none}");
    let paused = text(&state(cron(vec![view("a", "m", None)], running())), 100);
    assert!(paused.starts_with("⑤ ⏱ 1 cron jobs · clock"), "{paused}");
}

#[test]
fn the_earliest_active_slot_wins() {
    let jobs = vec![
        view("late", "x", Some(NOW + 7_200_000)),
        view("soon", "y", Some(NOW + 1_800_000)),
    ];
    let shown = text(&state(cron(jobs, running())), 160);
    assert!(shown.contains("next Mon 10-05 00:00 soon y"), "{shown}");
}

#[test]
fn failures_are_distinct_from_empty_and_keep_the_previous_jobs() {
    let failed = State {
        cron: None,
        failure: Some("storage unreachable\nsecond".into()),
    };
    let shown = text(&failed, 160);
    assert_eq!(shown, "⑤ ⏱ ✗ cron jobs unavailable: storage unreachable");
    assert_eq!(line(&State::default(), NOW, 160, Look::default()), None);
    let stale = State {
        cron: Some(cron(vec![view("a", "m", None)], running())),
        failure: Some("boom".into()),
    };
    assert!(text(&stale, 160).contains("1 cron jobs ! stale"));
}

#[test]
fn untrusted_text_is_neutralized_and_only_the_first_line_shows() {
    let message = "  investigate\u{1b}[31m\u{202e}界 e\u{301}\nsecond line  ";
    let job = view("o\u{1b}[2Jwner", message, Some(NOW + 60_000));
    let clock = ClockStatus::Running(Holder {
        pane: Some("%4\u{1b}[2J1".into()),
        pid: 1,
        since_ms: NOW,
        expires_ms: NOW,
    });
    let shown = text(&state(cron(vec![job], clock)), 160);
    assert!(
        !shown.contains('\u{1b}') && !shown.contains('\u{202e}'),
        "{shown}"
    );
    assert!(!shown.contains("second line"), "{shown}");
}

#[test]
fn every_width_stays_inside_the_available_cells() {
    let jobs = vec![view(
        "界界 e\u{301}",
        "界界 e\u{301} long text",
        Some(NOW + 1),
    )];
    let state = state(cron(jobs, ClockStatus::NoClock));
    for width in 0..=160 {
        let rendered = line(&state, NOW, width, Look::default()).unwrap();
        assert!(rendered.width() <= usize::from(width), "width {width}");
    }
}

#[test]
fn a_long_clock_holder_never_displaces_time_or_key() {
    let clock = ClockStatus::Running(Holder {
        pane: Some("holder界".repeat(30)),
        pid: 1,
        since_ms: NOW,
        expires_ms: NOW,
    });
    let state = state(cron(vec![view("a", "p", Some(NOW + 3_600_000))], clock));
    let shown = text(&state, 80);
    assert!(shown.contains("next Mon 10-05 00:30"), "{shown}");
    assert!(shown.ends_with("c list"), "{shown}");
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

fn snapshots() -> serde_json::Value {
    let job = view(
        "tmt-lead",
        "merge queue sweep and check every pending review",
        Some(NOW + 3_600_000),
    );
    let mut frames = Vec::new();
    for (scenario, state) in [
        ("running", self::state(cron(vec![job.clone()], running()))),
        (
            "no clock",
            self::state(cron(vec![job.clone()], ClockStatus::NoClock)),
        ),
        (
            "unknown",
            self::state(cron(vec![job], ClockStatus::Unknown)),
        ),
        ("empty", self::state(cron(vec![], ClockStatus::NoClock))),
        (
            "failed",
            State {
                cron: None,
                failure: Some("storage unreachable".into()),
            },
        ),
    ] {
        for width in [160u16, 100, 80] {
            frames.push(serde_json::json!({
                "scenario": scenario, "width": width, "line": text(&state, width),
            }));
        }
    }
    serde_json::json!(frames)
}

#[test]
fn home_line_snapshots_at_each_width() {
    assert_eq!(
        snapshots(),
        serde_json::from_str::<serde_json::Value>(include_str!("snapshots.json")).unwrap()
    );
}

#[test]
#[ignore = "explicit initial captures of the new ⑤ line; frozen parity is separate"]
fn record_home_line_snapshots() {
    std::fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/board/cronboard/line/snapshots.json"
        ),
        serde_json::to_string_pretty(&snapshots()).unwrap() + "\n",
    )
    .unwrap();
}
