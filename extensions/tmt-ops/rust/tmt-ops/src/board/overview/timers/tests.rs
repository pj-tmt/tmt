use super::*;
use crate::board::{
    app::workspace::tests::board,
    cronboard::{TEST_NOW, test_cron, test_views},
};
use tmt_ops::cron::{ClockStatus, Holder};
fn running() -> ClockStatus {
    ClockStatus::Running(Holder {
        pid: 7,
        pane: Some("%41".into()),
        since_ms: TEST_NOW,
        expires_ms: TEST_NOW + 5_000,
    })
}
#[test]
fn soonest_preview_is_bounded_deduplicated_and_keeps_other_room_ids() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = board();
    let mut jobs = test_views(5, "team", "room");
    for (job, minutes) in jobs.iter_mut().zip([49, 19, 21, 90, 100]) {
        job.next_ms = vec![TEST_NOW + minutes * 60_000];
    }
    jobs[4].owner_missing = true;
    jobs.push(jobs[1].clone());
    let mut other = jobs[1].clone();
    other.job.room_id = "other-room".into();
    other.job.squad = "other".into();
    other.next_ms = vec![TEST_NOW + 20 * 60_000];
    jobs.push(other);
    app.cron.replace(Ok(test_cron(jobs, running())));
    let model = project(&app);
    assert_eq!(model.total, 6);
    assert_eq!(model.rows.len(), 3);
    assert_eq!(
        model
            .rows
            .iter()
            .map(|r| (r.id.as_str(), r.squad.as_str(), r.next.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("c2", "team", Some("in 19m")),
            ("c2", "other", Some("in 20m")),
            ("c3", "team", Some("in 21m"))
        ]
    );
    assert!(model.note.starts_with("5 on · 1 paused"), "{}", model.note);
    let ids = model
        .rows
        .iter()
        .map(|r| (&r.id, &r.squad))
        .collect::<Vec<_>>();
    app.cron.replace(Err("unavailable".into()));
    let stale = project(&app);
    assert_eq!(
        stale
            .rows
            .iter()
            .map(|r| (&r.id, &r.squad))
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(stale.note, "stale");
    assert!(stale.rows.iter().all(|r| r.dim && r.next.is_none()));
}
#[test]
fn zero_unavailable_paused_and_missing_clock_remain_distinct() {
    let mut app = board();
    assert_eq!(project(&app).empty, "Timers unavailable");
    app.cron.replace(Ok(test_cron(vec![], running())));
    let empty = project(&app);
    assert_eq!(empty.empty, "No timers");
    assert_eq!(empty.note, "0 on · 0 paused");
    let mut jobs = test_views(2, "team", "room");
    jobs[1].owner_missing = true;
    app.cron.replace(Ok(test_cron(jobs, ClockStatus::NoClock)));
    let no_clock = project(&app);
    assert!(no_clock.rows[0].next.is_none());
    assert_eq!(no_clock.rows[1].next.as_deref(), Some("no owner"));
    assert!(no_clock.rows[1].dim);
    let mut jobs = test_views(3, "team", "room");
    jobs[1].job.pause = Some(tmt_ops::cron::Pause {
        by: "user-id".into(),
        at_ms: TEST_NOW,
    });
    jobs[2].job.owner_id = None;
    app.cron.replace(Ok(test_cron(jobs, running())));
    let paused = project(&app);
    assert_eq!(paused.note, "1 on · 1 paused · 1 no owner");
    assert_eq!(paused.rows[1].next.as_deref(), Some("paused"));
    assert!(paused.rows[1].dim);
    assert_eq!(paused.rows[2].next.as_deref(), Some("no owner"));
    assert!(paused.rows[2].dim);
    app.workspace.visibility.scope = vec!["room".into()];
    assert_eq!(project(&app).title, "Timers · global");
}
#[test]
fn countdown_is_snapshot_relative_and_never_fabricates_a_future_zero() {
    for (delta, text) in [
        (0, "due"),
        (1, "in <1m"),
        (19 * 60_000, "in 19m"),
        (3_600_000, "in 1h"),
        ((3 * 24 + 11) * 3_600_000, "in 3d11h"),
    ] {
        assert_eq!(cronboard::countdown(TEST_NOW + delta, TEST_NOW), text);
    }
}
