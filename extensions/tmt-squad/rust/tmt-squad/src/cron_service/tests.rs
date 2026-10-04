use super::test_support::*;
use super::*;
use std::fs;

#[test]
fn reads_are_unrestricted_and_actor_admission_never_falls_back_from_a_member() {
    let f = Fixture::new();
    assert!(
        list_jobs(&f.core, &f.config, None, 0)
            .unwrap()
            .jobs
            .is_empty()
    );
    assert!(!f.directory.join("squad").exists());
    let bytes = fs::read(f.config.path()).unwrap();
    f.change_model(|m| m["caller"] = json!(WORKER));
    let member = actor(&f.core, &f.config, None).unwrap();
    assert_eq!(member.id, WORKER);
    assert_eq!(
        f.add(&member, WORKER).err().unwrap().code,
        "SQUAD_CRON_PERMISSION_DENIED"
    );
    f.change_model(|m| m["caller"] = json!("ambiguous"));
    assert_eq!(
        actor(&f.core, &f.config, None).unwrap_err().code,
        "CALLER_IDENTITY_AMBIGUOUS"
    );
    let job = f.add(&f.actor(LEAD), WORKER).unwrap().job.job;
    assert_eq!(
        show_job(&f.core, &f.config, &JobKey::of(&job), 0)
            .unwrap()
            .next_ms
            .len(),
        3
    );
    assert_eq!(fs::read(f.config.path()).unwrap(), bytes);
}

#[test]
fn locked_revision_room_and_membership_guards_preserve_the_published_job() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let job = f.add(&user, WORKER).unwrap().job.job;
    let paused = f.mutate(&user, &job, Mutation::Pause).unwrap().job.job;
    let before = fs::read(f.directory.join("squad/cron/jobs.json")).unwrap();
    assert_eq!(
        f.mutate(&user, &job, Mutation::Resume).err().unwrap().code,
        "SQUAD_CRON_REVISION_CONFLICT"
    );
    assert_eq!(
        authorize_write(&f.core, &f.config, &JobKey::of(&job), &user, job.revision)
            .unwrap_err()
            .code,
        "SQUAD_CRON_REVISION_CONFLICT"
    );
    assert_eq!(
        admit_scheduled(&f.core, &JobKey::of(&paused), paused.revision)
            .unwrap_err()
            .code,
        "SQUAD_CRON_NOT_ON"
    );
    f.change_model(|m| m["room"] = json!("55555555-5555-4555-8555-555555555555"));
    assert_eq!(
        f.mutate(&user, &paused, Mutation::Resume)
            .err()
            .unwrap()
            .code,
        "SQUAD_CRON_ROOM_CONFLICT"
    );
    assert_eq!(
        f.add(&user, WORKER).err().unwrap().code,
        "SQUAD_CRON_ROOM_CONFLICT"
    );
    assert_eq!(
        fs::read(f.directory.join("squad/cron/jobs.json")).unwrap(),
        before
    );
    let calls = f.model()["calls"].as_array().unwrap().clone();
    assert!(
        calls
            .iter()
            .any(|call| call["locked"] == true && call["request"]["operation"] == "rooms.roster")
    );
}

#[test]
fn notices_target_owners_suppress_actor_and_never_undo_a_committed_change() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let lead = f.actor(LEAD);
    let job = f.add(&user, WORKER).unwrap().job.job;
    let paused = f.mutate(&user, &job, Mutation::Pause).unwrap().job.job;
    let resumed = f.mutate(&user, &paused, Mutation::Resume).unwrap().job.job;
    let reassigned = f
        .mutate(
            &user,
            &resumed,
            Mutation::Reassign {
                owner_id: LEAD.into(),
            },
        )
        .unwrap()
        .job
        .job;
    let notices = f.model()["notices"].as_array().unwrap().clone();
    assert_eq!(notices.len(), 5);
    assert_eq!(notices[3]["input"]["recipientIds"], json!([WORKER]));
    assert_eq!(notices[4]["input"]["recipientIds"], json!([LEAD]));
    assert!(
        notices[4]["input"]["message"]
            .as_str()
            .unwrap()
            .contains("literal {time}\\n!\\u{0} message")
    );
    for notice in &notices {
        assert_eq!(notice["input"]["kind"], "announcement");
        assert!(
            notice["input"]["message"]
                .as_str()
                .unwrap()
                .starts_with("▚ ⏱")
        );
        assert!(!notice["input"]["message"].as_str().unwrap().contains('\n'));
    }
    let paused = f
        .mutate(&lead, &reassigned, Mutation::Pause)
        .unwrap()
        .job
        .job;
    assert_eq!(f.model()["notices"].as_array().unwrap().len(), 5);
    assert!(!f.mutate(&lead, &paused, Mutation::Pause).unwrap().changed);
    f.change_model(|m| m["noticeFailure"] = json!(true));
    let applied = f.mutate(&user, &paused, Mutation::Resume).unwrap();
    assert_eq!(applied.job.job.state(), "on");
    assert_eq!(applied.warnings[0].code, "DISPATCH_FAILURE");
    assert_eq!(
        Store::new(&f.directory).unwrap().read().unwrap().jobs()[0],
        applied.job.job
    );
}

#[test]
fn retirement_is_durable_before_ack_and_obsolete_hooks_cannot_clear_a_new_owner() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let original = f.add(&user, WORKER).unwrap().job.job;
    let new = f
        .mutate(
            &user,
            &original,
            Mutation::Reassign {
                owner_id: LEAD.into(),
            },
        )
        .unwrap()
        .job
        .job;
    f.change_model(|m| m["retired"] = json!([WORKER]));
    assert!(drain_retired(&f.core, &f.config, 200).unwrap().is_empty());
    assert_eq!(
        Store::new(&f.directory).unwrap().read().unwrap().jobs()[0],
        new
    );
    let f = Fixture::new();
    let user = f.actor(USER);
    let owned = f.add(&user, WORKER).unwrap().job.job;
    f.change_model(|m| m["retired"] = json!([WORKER]));
    let view = show_job(&f.core, &f.config, &JobKey::of(&owned), 300).unwrap();
    assert_eq!(view.job.state(), "no owner");
    assert_eq!(view.job.revision, owned.revision + 1);
    assert_eq!(view.job.pause.as_ref().unwrap().at_ms, 300);
    assert!(f.model()["hooks"].as_array().unwrap().is_empty());
    let notices = f.model()["notices"].as_array().unwrap().clone();
    assert_eq!(
        notices.last().unwrap()["input"]["recipientIds"],
        json!([LEAD])
    );
    assert_eq!(notices.last().unwrap()["originator"], "anonymous");
    assert!(drain_retired(&f.core, &f.config, 400).unwrap().is_empty());
    assert_eq!(
        f.model()["notices"].as_array().unwrap().len(),
        notices.len()
    );
    assert_eq!(
        f.mutate(&user, &view.job, Mutation::Resume)
            .err()
            .unwrap()
            .code,
        "SQUAD_CRON_NO_OWNER"
    );
}

#[test]
fn failed_hook_registration_rolls_back_and_non_members_cannot_own_jobs() {
    let f = Fixture::new();
    let user = f.actor(USER);
    f.change_model(|m| m["hookFailure"] = json!(true));
    assert_eq!(f.add(&user, WORKER).err().unwrap().code, "HOOK_FAILURE");
    assert!(
        Store::new(&f.directory)
            .unwrap()
            .read()
            .unwrap()
            .jobs()
            .is_empty()
    );
    f.change_model(|m| m["hookFailure"] = json!(false));
    let job = f.add(&user, WORKER).unwrap().job.job;
    assert_eq!(job.id(), "c1");
    f.change_model(|m| m["rosterWithout"] = json!([WORKER]));
    assert_eq!(
        admit_scheduled(&f.core, &JobKey::of(&job), job.revision)
            .unwrap_err()
            .code,
        "SQUAD_NOT_A_MEMBER"
    );
    let empty = f.mutate(
        &user,
        &job,
        Mutation::Edit {
            message: Some(" \n".into()),
            schedule: None,
        },
    );
    assert_eq!(empty.err().unwrap().code, "SQUAD_CRON_MESSAGE_INVALID");
    assert_eq!(
        Store::new(&f.directory).unwrap().read().unwrap().jobs()[0]
            .owner_id
            .as_deref(),
        Some(WORKER)
    );
}
