use super::*;
use crate::cron_service::{
    self, Change, Mutation,
    test_support::{Fixture, ROOM, USER, WORKER},
};

fn request(repair: bool) -> Rename {
    Rename {
        room_id: ROOM.into(),
        old: "product".into(),
        new: "renamed".into(),
        expected_revision: Some(1),
        repair,
    }
}
fn room_writes(fixture: &Fixture) -> usize {
    fixture.model()["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|call| call["request"]["operation"] == "rooms.write")
        .count()
}
fn config(fixture: &Fixture) -> Config {
    Config::read(fixture.config.path().to_path_buf()).unwrap()
}
fn add_after_rename(fixture: &Fixture, config: &Config) -> String {
    let actor = fixture.actor(USER);
    let schedule = tmt_ops::cron::Schedule::parse(
        tmt_ops::cron::ScheduleInput::Every {
            duration: "30m",
            from: None,
        },
        "UTC",
        0,
    )
    .unwrap();
    cron_service::apply(
        &fixture.core,
        config,
        &actor,
        Change::Add {
            squad: "renamed".into(),
            room_id: ROOM.into(),
            owner_id: WORKER.into(),
            message: "next".into(),
            schedule,
            paused: false,
        },
        0,
    )
    .unwrap()
    .job
    .job
    .id()
}

#[test]
fn rename_lists_same_jobs_and_add_continues_the_original_counter() {
    let f = Fixture::new();
    let job = f.add(&f.actor(USER), WORKER).unwrap().job.job;
    let paused = f
        .mutate(&f.actor(USER), &job, Mutation::Pause)
        .unwrap()
        .job
        .job;
    fs::write(
        f.config.path(),
        format!("# global\nme='Ben'\nme_id='{USER}'\n[squad.product]\nlayout='crew'\n"),
    )
    .unwrap();
    f.change_model(|model| {
        model["members"][2]["metadata"] =
            json!({"squad.product.state":"working", "squad.other.task":"kept"});
    });
    let mut config = config(&f);
    let outcome = rename_squad(&f.core, &mut config, &request(false)).unwrap();
    assert!(outcome.complete);
    assert_eq!(room_writes(&f), 1);
    assert_eq!(f.model()["roomName"], "squad-renamed");
    let listed = cron_service::list_jobs(&f.core, &config, Some("renamed"), 0).unwrap();
    assert_eq!(listed.jobs.len(), 1);
    assert_eq!(listed.jobs[0].job.room_id, paused.room_id);
    assert_eq!(listed.jobs[0].job.id(), paused.id());
    assert_eq!(listed.jobs[0].job.pause, paused.pause);
    assert_eq!(listed.jobs[0].job.squad, "renamed");
    assert_eq!(add_after_rename(&f, &config), "c2");
    assert!(config.squad_config_text("product").is_none());
    assert!(config.squad_config_text("renamed").is_some());
    assert_eq!(
        f.model()["members"][2]["metadata"],
        json!({"squad.renamed.state":"working", "squad.other.task":"kept"})
    );
    let hooks = f.model()["hooks"].as_array().unwrap().clone();
    assert!(
        hooks.iter().any(|hook| hook["identityId"] == WORKER
            && hook["reference"] == format!("{ROOM}:renamed:c1"))
    );
}

#[test]
fn rename_after_removing_all_jobs_still_allocates_the_next_id() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let job = f.add(&user, WORKER).unwrap().job.job;
    f.mutate(&user, &job, Mutation::Remove).unwrap();
    let mut config = config(&f);
    assert!(
        rename_squad(&f.core, &mut config, &request(false))
            .unwrap()
            .complete
    );
    assert_eq!(add_after_rename(&f, &config), "c2");
}

#[test]
fn interrupted_second_step_prints_exact_repair_and_never_repeats_room_cas() {
    let f = Fixture::new();
    f.add(&f.actor(USER), WORKER).unwrap();
    f.change_model(|model| {
        model["members"][2]["metadata"] = json!({"squad.product.task":"keep draft"});
        model["applyFailure"] = json!("STORAGE_UNAVAILABLE");
    });
    let mut config = config(&f);
    let failed = rename_squad(&f.core, &mut config, &request(false)).unwrap();
    assert!(!failed.complete);
    assert_eq!(failed.document["roomStatus"], "Applied");
    assert_eq!(
        failed.document["repair"],
        format!("tmt ops squad rename product renamed --room-id {ROOM} --repair")
    );
    assert_eq!(room_writes(&f), 1);
    assert_eq!(f.model()["roomName"], "squad-renamed");
    assert_eq!(
        f.model()["members"][2]["metadata"]["squad.product.task"],
        "keep draft"
    );
    f.change_model(|model| model["applyFailure"] = json!(false));
    let repaired = rename_squad(&f.core, &mut config, &request(true)).unwrap();
    assert!(repaired.complete);
    assert_eq!(room_writes(&f), 1);
    assert_eq!(
        f.model()["members"][2]["metadata"]["squad.renamed.task"],
        "keep draft"
    );
    assert_eq!(add_after_rename(&f, &config), "c2");
}

#[test]
fn uncertain_room_response_requires_explicit_repair_without_duplicate_write() {
    let f = Fixture::new();
    f.change_model(|model| model["loseRoomWriteResponse"] = json!(true));
    let mut config = config(&f);
    let outcome = rename_squad(&f.core, &mut config, &request(false)).unwrap();
    assert!(!outcome.complete);
    assert_eq!(outcome.document["roomStatus"], "Unknown");
    assert_eq!(room_writes(&f), 1);
    assert_eq!(f.model()["roomName"], "squad-renamed");
    assert_eq!(
        rename_squad(&f.core, &mut config, &request(false))
            .err()
            .unwrap()
            .code,
        "SQUAD_RENAME_PENDING"
    );
    assert_eq!(room_writes(&f), 1);
    assert!(
        rename_squad(&f.core, &mut config, &request(true))
            .unwrap()
            .complete
    );
    assert_eq!(room_writes(&f), 1);
}

#[test]
fn malformed_success_is_not_a_confirmation_and_pending_hint_uses_retained_names() {
    let f = Fixture::new();
    f.change_model(|model| model["malformedRoomWriteResponse"] = json!(true));
    let mut config = config(&f);
    let outcome = rename_squad(&f.core, &mut config, &request(false)).unwrap();
    assert!(!outcome.complete);
    assert_eq!(outcome.document["roomStatus"], "Unknown");
    let mut other = request(false);
    other.new = "another".into();
    let pending = rename_squad(&f.core, &mut config, &other).err().unwrap();
    assert_eq!(pending.code, "SQUAD_RENAME_PENDING");
    assert_eq!(
        pending.human().1,
        Some(repair_command(&request(true)).as_str())
    );
    assert!(
        rename_squad(&f.core, &mut config, &request(true))
            .unwrap()
            .complete
    );
    assert_eq!(room_writes(&f), 1);
}

#[test]
fn unconfirmed_metadata_exchange_is_partial_and_repair_observes_the_recorded_exchange() {
    let f = Fixture::new();
    f.change_model(|model| {
        model["members"][1]["metadata"] = json!({});
        model["members"][2]["metadata"] = json!({"squad.product.task":"keep"});
        model["malformedApplyResponse"] = json!(true);
    });
    let mut config = config(&f);
    let outcome = rename_squad(&f.core, &mut config, &request(false)).unwrap();
    assert!(!outcome.complete);
    assert_eq!(outcome.document["failed"]["stage"], "metadata");
    assert_eq!(
        f.model()["members"][2]["metadata"],
        json!({"squad.renamed.task":"keep"})
    );
    assert!(
        rename_squad(&f.core, &mut config, &request(true))
            .unwrap()
            .complete
    );
    assert_eq!(room_writes(&f), 1);
}

#[test]
fn frozen_revision_and_destination_namespace_fail_before_any_effect() {
    let f = Fixture::new();
    let mut config = config(&f);
    let mut stale = request(false);
    stale.expected_revision = Some(9);
    assert_eq!(
        rename_squad(&f.core, &mut config, &stale)
            .err()
            .unwrap()
            .code,
        "ROOM_REVISION_CONFLICT"
    );
    assert_eq!(room_writes(&f), 0);
    let store = tmt_ops::cron::Store::new(&f.directory.join("ops")).unwrap();
    store
        .update(|jobs| {
            let schedule = tmt_ops::cron::Schedule::parse(
                tmt_ops::cron::ScheduleInput::Every {
                    duration: "30m",
                    from: None,
                },
                "UTC",
                0,
            )?;
            jobs.insert(tmt_ops::cron::Job::new(
                "product".into(),
                ROOM.into(),
                WORKER.into(),
                "old".into(),
                schedule.clone(),
            ))?;
            jobs.insert(tmt_ops::cron::Job::new(
                "renamed".into(),
                "other-room".into(),
                WORKER.into(),
                "other".into(),
                schedule,
            ))?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        rename_squad(&f.core, &mut config, &request(false))
            .err()
            .unwrap()
            .code,
        "SQUAD_CRON_NAMESPACE_CONFLICT"
    );
    assert_eq!(room_writes(&f), 0);
}

#[test]
fn explicit_repair_refuses_conflicting_metadata_instead_of_overwriting_it() {
    let f = Fixture::new();
    f.change_model(|model| {
        model["members"][2]["metadata"] = json!({"squad.product.task":"original"});
        model["applyFailure"] = json!("STORAGE_UNAVAILABLE");
    });
    let mut config = config(&f);
    assert!(
        !rename_squad(&f.core, &mut config, &request(false))
            .unwrap()
            .complete
    );
    f.change_model(|model| {
        model["applyFailure"] = json!(false);
        model["members"][2]["metadata"]["squad.product.task"] = json!("new user value");
    });
    let outcome = rename_squad(&f.core, &mut config, &request(true)).unwrap();
    assert!(!outcome.complete);
    assert_eq!(
        outcome.document["failed"]["error"]["code"],
        "METADATA_CONFLICT"
    );
    assert_eq!(
        f.model()["members"][2]["metadata"]["squad.product.task"],
        "new user value"
    );
    assert_eq!(room_writes(&f), 1);
}
