use super::*;
use crate::cron_service::{list_jobs, test_support::*};
use std::fs;

const NOW: i64 = 1_700_000_000_000;

fn add(f: &Fixture, actor: &str, message: &str) -> Result<String, crate::core::SquadError> {
    run(
        &f.core,
        &f.config,
        CronRequest::Add {
            actor: f.actor(actor),
            squad: "product".into(),
            room_id: ROOM.into(),
            owner: "worker".into(),
            message: message.into(),
            schedule: "every 30m".into(),
            zone: "UTC".into(),
        },
        NOW,
    )
}

fn only(f: &Fixture) -> crate::cron_service::JobView {
    let mut jobs = list_jobs(&f.core, &f.config, None, NOW).unwrap().jobs;
    assert_eq!(jobs.len(), 1);
    jobs.remove(0)
}

fn existing(
    f: &Fixture,
    actor: &str,
    job: &crate::cron_service::JobView,
    revision: u64,
    op: Op,
) -> CronRequest {
    CronRequest::Existing {
        actor: f.actor(actor),
        key: JobKey::of(&job.job),
        revision,
        op,
    }
}

fn store(f: &Fixture) -> Vec<u8> {
    fs::read(f.directory.join("ops/cron/jobs.json")).unwrap()
}

#[test]
fn a_form_to_the_store_keeps_the_exact_message_and_every_control_is_durable() {
    let f = Fixture::new();
    let message = "  keep {this} \u{1f600}  ";
    let done = add(&f, USER, message).unwrap();
    assert!(
        done.starts_with("Added product c1 (every 30m) · owner worker"),
        "{done}"
    );
    let job = only(&f);
    assert_eq!(job.job.message, message, "no trim, no substitution");
    let revision = job.job.revision;
    let edit = Op::Edit {
        message: None,
        schedule: Some("daily 09:00".into()),
        zone: "UTC".into(),
    };
    run(
        &f.core,
        &f.config,
        existing(&f, USER, &job, revision, edit),
        NOW,
    )
    .unwrap();
    let edited = only(&f);
    assert_eq!(
        edited.job.message, message,
        "an untouched message stays as stored"
    );
    assert_eq!(
        crate::cron_service::schedule_text(&edited.job.schedule),
        "daily 09:00"
    );
    let paused = run(
        &f.core,
        &f.config,
        existing(&f, USER, &edited, edited.job.revision, Op::Pause),
        NOW,
    )
    .unwrap();
    assert!(paused.starts_with("Paused product c1"), "{paused}");
    assert_eq!(only(&f).job.state(), "paused");
    let again = run(
        &f.core,
        &f.config,
        existing(&f, USER, &only(&f), only(&f).job.revision, Op::Pause),
        NOW,
    )
    .unwrap();
    assert!(again.contains("already as requested"), "{again}");
    let current = only(&f);
    run(
        &f.core,
        &f.config,
        existing(&f, USER, &current, current.job.revision, Op::Resume),
        NOW,
    )
    .unwrap();
    let current = only(&f);
    let to = Op::Reassign {
        owner: "Sol".into(),
    };
    let moved = run(
        &f.core,
        &f.config,
        existing(&f, USER, &current, current.job.revision, to),
        NOW,
    )
    .unwrap();
    assert!(moved.contains("owner Sol"), "{moved}");
    let current = only(&f);
    run(
        &f.core,
        &f.config,
        existing(&f, USER, &current, current.job.revision, Op::Remove),
        NOW,
    )
    .unwrap();
    assert!(
        list_jobs(&f.core, &f.config, None, NOW)
            .unwrap()
            .jobs
            .is_empty()
    );
}

#[test]
fn stale_unauthorized_and_unknown_owner_submissions_write_nothing() {
    let f = Fixture::new();
    add(&f, USER, "m").unwrap();
    let job = only(&f);
    let first = job.job.revision;
    run(
        &f.core,
        &f.config,
        existing(&f, USER, &job, first, Op::Pause),
        NOW,
    )
    .unwrap();
    let before = store(&f);
    // The board viewed revision `first`; the job has since changed.
    let stale = run(
        &f.core,
        &f.config,
        existing(&f, USER, &job, first, Op::Resume),
        NOW,
    )
    .unwrap_err();
    assert_eq!(stale.code, "SQUAD_CRON_REVISION_CONFLICT");
    let current = only(&f).job.revision;
    let denied = run(
        &f.core,
        &f.config,
        existing(
            &f,
            "33333333-3333-4333-8333-333333333333",
            &job,
            current,
            Op::Resume,
        ),
        NOW,
    )
    .unwrap_err();
    assert_eq!(denied.code, "SQUAD_CRON_PERMISSION_DENIED");
    let nobody = Op::Reassign {
        owner: "nobody".into(),
    };
    assert!(
        run(
            &f.core,
            &f.config,
            existing(&f, USER, &job, current, nobody),
            NOW
        )
        .is_err()
    );
    let bad = Op::Edit {
        message: None,
        schedule: Some("whenever".into()),
        zone: "UTC".into(),
    };
    assert_eq!(
        run(
            &f.core,
            &f.config,
            existing(&f, USER, &job, current, bad),
            NOW
        )
        .unwrap_err()
        .code,
        "SQUAD_CRON_SCHEDULE_INVALID"
    );
    assert_eq!(
        store(&f),
        before,
        "refused requests leave the store byte for byte"
    );
    // A job that is gone is reported, not recreated.
    run(
        &f.core,
        &f.config,
        existing(&f, USER, &job, current, Op::Remove),
        NOW,
    )
    .unwrap();
    let gone = run(
        &f.core,
        &f.config,
        existing(&f, USER, &job, current + 1, Op::Pause),
        NOW,
    )
    .unwrap_err();
    assert_eq!(gone.code, "SQUAD_CRON_NOT_FOUND");
}

#[test]
fn an_empty_message_and_a_member_adding_are_refused_without_a_job() {
    let f = Fixture::new();
    assert_eq!(
        add(&f, USER, "   ").unwrap_err().code,
        "SQUAD_CRON_MESSAGE_INVALID"
    );
    let worker = "33333333-3333-4333-8333-333333333333";
    assert_eq!(
        add(&f, worker, "m").unwrap_err().code,
        "SQUAD_CRON_PERMISSION_DENIED"
    );
    assert!(
        list_jobs(&f.core, &f.config, None, NOW)
            .unwrap()
            .jobs
            .is_empty()
    );
}

#[test]
fn send_now_is_one_acceptance_per_action_with_the_exact_message_and_works_when_paused() {
    let f = Fixture::new();
    let message = "  exact {now} \n !\0";
    add(&f, USER, message).unwrap();
    let job = only(&f);
    let done = run(
        &f.core,
        &f.config,
        existing(&f, USER, &job, job.job.revision, Op::Send),
        NOW,
    )
    .unwrap();
    assert!(done.starts_with("Accepted product c1"), "{done}");
    assert!(done.contains("delivery is not confirmed"), "{done}");
    run(
        &f.core,
        &f.config,
        existing(&f, USER, &job, job.job.revision, Op::Pause),
        NOW,
    )
    .unwrap();
    let paused = only(&f);
    run(
        &f.core,
        &f.config,
        existing(&f, USER, &paused, paused.job.revision, Op::Send),
        NOW,
    )
    .unwrap();
    let model = f.model();
    let dispatches = model["dispatches"].as_object().unwrap();
    assert_eq!(dispatches.len(), 2, "each explicit send is its own action");
    for sent in dispatches.values() {
        assert_eq!(sent["intent"]["input"]["message"], message);
    }
    assert_eq!(
        only(&f).job.state(),
        "paused",
        "sending changes no schedule or state"
    );
    // A stale revision is refused before anything is sent.
    let before = f.model()["dispatches"].as_object().unwrap().len();
    let stale = run(
        &f.core,
        &f.config,
        existing(&f, USER, &job, job.job.revision, Op::Send),
        NOW,
    )
    .unwrap_err();
    assert_eq!(stale.code, "SQUAD_CRON_REVISION_CONFLICT");
    assert_eq!(f.model()["dispatches"].as_object().unwrap().len(), before);
}
