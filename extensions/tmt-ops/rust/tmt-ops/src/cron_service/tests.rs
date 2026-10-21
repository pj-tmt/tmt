use super::test_support::*;
use super::*;
use crate::test_support::{replace_stored_jobs, stored_jobs};
use std::fs;

#[test]
fn missing_squad_owner_is_listed_paused_and_readding_it_does_not_resume() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let job = f.add(&user, WORKER).unwrap().job.job;
    f.change_model(|model| model["rosterWithout"] = json!([WORKER]));
    let listed = list_jobs(&f.core, &f.config, Some("product"), 100).unwrap();
    assert_eq!(listed.jobs.len(), 1);
    let missing = &listed.jobs[0];
    assert!(missing.owner_missing);
    assert_eq!(missing.state_label(), "paused · owner missing");
    assert_eq!(missing.job.owner_id.as_deref(), Some(WORKER));
    assert_eq!(missing.job.pause.as_ref().unwrap().at_ms, 100);
    assert!(missing.next_ms.is_empty());
    assert_eq!(missing.job.revision, job.revision + 1);
    assert_eq!(missing.document()["ownerMissing"], true);
    f.change_model(|model| model["rosterWithout"] = json!([]));
    let restored = show_job(&f.core, &f.config, &JobKey::of(&job), 200).unwrap();
    assert!(!restored.owner_missing);
    assert_eq!(restored.job.pause, missing.job.pause);
    assert_eq!(restored.job.revision, missing.job.revision);
    assert_eq!(
        admit_scheduled(&f.core, &JobKey::of(&job), restored.job.revision)
            .unwrap_err()
            .code,
        "SQUAD_CRON_NOT_ON"
    );
    assert!(
        f.mutate(&user, &restored.job, Mutation::Resume)
            .unwrap()
            .job
            .job
            .pause
            .is_none()
    );
}

#[test]
fn scheduled_admission_and_explicit_resume_persist_missing_owner_pause_before_refusing() {
    for resume in [false, true] {
        let f = Fixture::new();
        let user = f.actor(USER);
        let job = f.add(&user, WORKER).unwrap().job.job;
        f.change_model(|model| model["rosterWithout"] = json!([WORKER]));
        let error = if resume {
            f.mutate(&user, &job, Mutation::Resume).err().unwrap()
        } else {
            admit_scheduled(&f.core, &JobKey::of(&job), job.revision).unwrap_err()
        };
        assert_eq!(error.code, "SQUAD_NOT_A_MEMBER");
        let stored = Store::new(&f.directory.join("ops"))
            .unwrap()
            .read()
            .unwrap();
        let paused = &stored.jobs()[0];
        assert!(paused.pause.is_some());
        assert_eq!(paused.revision, job.revision + 1);
        let reassigned = f
            .mutate(
                &user,
                paused,
                Mutation::Reassign {
                    owner_id: LEAD.into(),
                },
            )
            .unwrap()
            .job
            .job;
        assert_eq!(reassigned.pause, paused.pause);
        assert_eq!(reassigned.owner_id.as_deref(), Some(LEAD));
    }
}

#[test]
fn direct_reassignment_of_a_missing_owner_keeps_a_pause_until_explicit_resume() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let job = f.add(&user, WORKER).unwrap().job.job;
    f.change_model(|model| model["rosterWithout"] = json!([WORKER]));
    let reassigned = f
        .mutate(
            &user,
            &job,
            Mutation::Reassign {
                owner_id: LEAD.into(),
            },
        )
        .unwrap()
        .job
        .job;
    assert!(reassigned.pause.is_some());
    assert_eq!(reassigned.revision, job.revision + 1);
    assert!(
        f.mutate(&user, &reassigned, Mutation::Resume)
            .unwrap()
            .job
            .job
            .pause
            .is_none()
    );
}

#[test]
fn failed_or_incomplete_owner_observation_never_claims_missing_or_changes_jobs() {
    for malformed in [false, true] {
        let f = Fixture::new();
        let job = f.add(&f.actor(USER), WORKER).unwrap().job.job;
        let before = stored_jobs(&f.directory);
        f.change_model(|model| {
            if malformed {
                model["malformedReferences"] = json!(true);
            } else {
                model["failOperation"] = json!("rooms.roster");
            }
        });
        assert!(list_jobs(&f.core, &f.config, Some("product"), 100).is_err());
        assert!(admit_scheduled(&f.core, &JobKey::of(&job), job.revision).is_err());
        assert_eq!(stored_jobs(&f.directory), before);
    }
}

#[test]
fn reads_are_unrestricted_and_actor_admission_never_falls_back_from_a_member() {
    let f = Fixture::new();
    assert!(
        list_jobs(&f.core, &f.config, None, 0)
            .unwrap()
            .jobs
            .is_empty()
    );
    assert!(!f.directory.join("ops").exists());
    let bytes = fs::read(f.config.path()).unwrap();
    f.change_model(|m| m["caller"] = json!(WORKER));
    let member = actor(&f.core, None).unwrap();
    assert_eq!(member.id, WORKER);
    assert_eq!(
        f.add(&member, WORKER).err().unwrap().code,
        "SQUAD_CRON_PERMISSION_DENIED"
    );
    f.change_model(|m| m["caller"] = json!("ambiguous"));
    assert_eq!(
        actor(&f.core, None).unwrap_err().code,
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
    let before = stored_jobs(&f.directory);
    assert_eq!(
        f.mutate(&user, &job, Mutation::Resume).err().unwrap().code,
        "SQUAD_CRON_REVISION_CONFLICT"
    );
    assert_eq!(
        authorize_write(&f.core, &JobKey::of(&job), &user, job.revision)
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
    assert_eq!(stored_jobs(&f.directory), before);
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
        Store::new(&f.directory.join("ops"))
            .unwrap()
            .read()
            .unwrap()
            .jobs()[0],
        applied.job.job
    );
}

#[test]
fn change_announcements_declare_automation_and_only_that_refusal_is_retried() {
    let dispatches = |f: &Fixture| {
        f.model()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|call| call["request"]["operation"] == "dispatch.create")
            .count()
    };
    let f = Fixture::new();
    let user = f.actor(USER);
    let job = f.add(&user, WORKER).unwrap().job.job;
    let model = f.model();
    assert_eq!(model["notices"].as_array().unwrap().len(), 1);
    assert_eq!(model["notices"][0]["input"]["origin"], "automation");

    // A Core from before origins refuses the field; the notice is sent again without it.
    f.change_model(|m| m["rejectOrigin"] = json!(true));
    let paused = f.mutate(&user, &job, Mutation::Pause).unwrap();
    assert!(paused.warnings.is_empty());
    let model = f.model();
    assert_eq!(model["notices"].as_array().unwrap().len(), 2);
    assert!(model["notices"][1]["input"].get("origin").is_none());

    // Any other refusal is reported once, not retried.
    f.change_model(|m| {
        m["rejectOrigin"] = json!(false);
        m["noticeFailure"] = json!(true);
    });
    let before = dispatches(&f);
    let resumed = f.mutate(&user, &paused.job.job, Mutation::Resume).unwrap();
    assert_eq!(resumed.warnings[0].code, "DISPATCH_FAILURE");
    assert_eq!(dispatches(&f), before + 1);
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
        Store::new(&f.directory.join("ops"))
            .unwrap()
            .read()
            .unwrap()
            .jobs()[0],
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
        Store::new(&f.directory.join("ops"))
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
    let paused = Store::new(&f.directory.join("ops"))
        .unwrap()
        .read()
        .unwrap()
        .jobs()[0]
        .clone();
    assert_eq!(paused.revision, job.revision + 1);
    assert!(paused.pause.is_some());
    let empty = f.mutate(
        &user,
        &paused,
        Mutation::Edit {
            message: Some(" \n".into()),
            schedule: None,
        },
    );
    assert_eq!(empty.err().unwrap().code, "SQUAD_CRON_MESSAGE_INVALID");
    assert_eq!(
        Store::new(&f.directory.join("ops"))
            .unwrap()
            .read()
            .unwrap()
            .jobs()[0]
            .owner_id
            .as_deref(),
        Some(WORKER)
    );
}

#[test]
fn schedule_text_round_trips_through_the_editor_grammar() {
    let now = 1_700_000_000_000;
    for text in [
        "every 30m",
        "every 3h from 09:00",
        "daily 23:00",
        "weekdays 09:00",
        "mon,thu 10:00",
        "cron 0 */3 * * *",
    ] {
        let parsed = parse_schedule_text(text, "Asia/Tokyo", now).unwrap();
        assert_eq!(schedule_text(&parsed), text, "{text}");
    }
    // A bare five-field expression is the same schedule as the shown `cron …` form.
    assert_eq!(
        parse_schedule_text(" 0  */3 * * * ", "Asia/Tokyo", now).unwrap(),
        parse_schedule_text("cron 0 */3 * * *", "Asia/Tokyo", now).unwrap()
    );
    for text in [
        "",
        "soon",
        "every",
        "every 0m",
        "daily",
        "daily 25:00",
        "0 0 *",
    ] {
        assert_eq!(
            parse_schedule_text(text, "Asia/Tokyo", now)
                .unwrap_err()
                .code,
            "SQUAD_CRON_SCHEDULE_INVALID",
            "{text:?}"
        );
    }
}

#[test]
fn departure_pause_survives_readd_and_reassign_until_explicit_resume() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let job = f.add(&user, WORKER).unwrap().job.job;
    assert_eq!(
        with_departure_pause(&f.core, ROOM, WORKER, 500, || ())
            .unwrap()
            .0,
        [job.id()]
    );
    f.change_model(|model| model["rosterWithout"] = json!([WORKER]));
    let stored = store(&f.core).unwrap().0.read().unwrap().jobs()[0].clone();
    assert_eq!(stored.state(), "paused");
    f.change_model(|model| model["rosterWithout"] = json!([]));
    let seen = show_job(&f.core, &f.config, &JobKey::of(&stored), 600).unwrap();
    assert_eq!(seen.job.state(), "paused");
    assert!(seen.next_ms.is_empty());
    let reassigned = f
        .mutate(
            &user,
            &stored,
            Mutation::Reassign {
                owner_id: LEAD.into(),
            },
        )
        .unwrap()
        .job
        .job;
    assert_eq!(reassigned.pause, stored.pause);
    assert_eq!(reassigned.state(), "paused");
    let resumed = f
        .mutate(&user, &reassigned, Mutation::Resume)
        .unwrap()
        .job
        .job;
    assert_eq!(resumed.state(), "on");
}

#[test]
fn membership_service_pauses_before_leave_and_keeps_other_metadata() {
    let f = Fixture::new();
    let user = f.actor(USER);
    f.add(&user, WORKER).unwrap();
    f.change_model(|model| {
        model["members"][2]["metadata"] = json!({"squad.product.state":"working", "squad.other.task":"kept", "task":"identity field"});
    });
    let selected = Squad::resolve(&f.core, Some("product")).unwrap();
    let result = crate::membership::remove(&f.core, &selected, WORKER).unwrap();
    assert!(result.complete);
    assert_eq!(result.document["pausedTimers"], json!(["c1"]));
    assert_eq!(f.model()["jobsAtLeave"]["jobs"][0]["state"], "paused");
    assert_eq!(result.document["cleared"], json!(["state"]));
    assert_eq!(
        store(&f.core).unwrap().0.read().unwrap().jobs()[0].state(),
        "paused"
    );
    assert_eq!(
        f.model()["members"][2]["metadata"],
        json!({"squad.other.task":"kept", "task":"identity field"})
    );
}

#[test]
fn failed_pause_publication_refuses_membership_leave() {
    let f = Fixture::new();
    f.add(&f.actor(USER), WORKER).unwrap();
    // A real store failure must happen before the public leave command.
    replace_stored_jobs(&f.directory, b"not a cron document");
    let selected = Squad {
        name: "product".into(),
        room_id: ROOM.into(),
    };
    let error = crate::membership::remove(&f.core, &selected, WORKER)
        .err()
        .unwrap();
    assert_eq!(error.code, "SQUAD_CRON_STORE_INVALID");
    assert!(f.model()["rosterWithout"].as_array().unwrap().is_empty());
    assert!(
        !f.model()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|call| call["command"][0] == "room" && call["command"][1] == "leave")
    );
}

#[test]
fn a_busy_store_names_its_cause_and_the_one_retry() {
    let what =
        "Another Ops process still holds the cron store lock after 5 s, so nothing was saved.";
    let error = SquadError::from(cron::Error {
        code: cron::STORE_BUSY.into(),
        message: what.into(),
    });
    assert_eq!(error.code, "SQUAD_CRON_STORE_BUSY");
    assert_eq!(error.human(), (what, Some("Run the same command again.")));
    assert_eq!(error.message, format!("{what} Run the same command again."));
}

#[test]
fn lost_leave_response_keeps_pause_and_reports_unknown_without_replaying() {
    let f = Fixture::new();
    f.add(&f.actor(USER), WORKER).unwrap();
    f.change_model(|model| model["loseLeaveResponse"] = json!(true));
    let selected = Squad {
        name: "product".into(),
        room_id: ROOM.into(),
    };
    let outcome = crate::membership::remove(&f.core, &selected, WORKER).unwrap();
    assert!(!outcome.complete);
    assert_eq!(outcome.document["failed"]["status"], "Unknown");
    assert!(outcome.document["removed"].is_null());
    assert_eq!(outcome.document["pausedTimers"], json!(["c1"]));
    assert_eq!(
        f.model()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|call| call["command"][0] == "room" && call["command"][1] == "leave")
            .count(),
        1
    );
    assert_eq!(
        store(&f.core).unwrap().0.read().unwrap().jobs()[0].state(),
        "paused"
    );
}

#[test]
fn metadata_failure_after_leave_reports_the_confirmed_removal_and_retained_pause() {
    let f = Fixture::new();
    f.add(&f.actor(USER), WORKER).unwrap();
    f.change_model(|model| {
        model["members"][2]["metadata"] = json!({"squad.product.state":"working"});
        model["metaRemoveFailure"] = json!("STORAGE_UNAVAILABLE");
    });
    let selected = Squad {
        name: "product".into(),
        room_id: ROOM.into(),
    };
    let outcome = crate::membership::remove(&f.core, &selected, WORKER).unwrap();
    assert!(!outcome.complete);
    assert_eq!(outcome.document["removed"]["id"], WORKER);
    assert_eq!(outcome.document["failed"]["stage"], "metadata clear");
    assert_eq!(outcome.document["failed"]["status"], "Unknown");
    assert_eq!(outcome.document["pausedTimers"], json!(["c1"]));
    assert_eq!(f.model()["rosterWithout"], json!([WORKER]));
}

#[test]
fn member_display_rename_refreshes_owner_name_without_moving_timer_state() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let job = f.add(&user, WORKER).unwrap().job.job;
    let before = stored_jobs(&f.directory);
    f.change_model(|model| {
        let member = model["members"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|member| member["id"] == WORKER)
            .unwrap();
        member["name"] = json!("renamed-worker");
    });
    let listed = list_jobs(&f.core, &f.config, Some("product"), 100).unwrap();
    assert_eq!(listed.jobs.len(), 1);
    let current = &listed.jobs[0];
    assert_eq!(current.owner_name.as_deref(), Some("renamed-worker"));
    assert_eq!(current.job, job);
    assert!(!current.owner_missing);
    assert_eq!(stored_jobs(&f.directory), before);
    let next = f.add(&user, WORKER).unwrap().job.job;
    assert_eq!(next.id(), "c2");
    assert_eq!(next.owner_id.as_deref(), Some(WORKER));
    assert_eq!(next.room_id, job.room_id);
}
