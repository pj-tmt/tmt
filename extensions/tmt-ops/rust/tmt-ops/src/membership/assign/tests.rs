use super::*;
use crate::cron_service::{
    self, Mutation,
    test_support::{Fixture, ROOM, USER, WORKER},
};

const DESTINATION: &str = "55555555-5555-4555-8555-555555555555";
fn squads(f: &Fixture) -> (Squad, Squad) {
    f.change_model(|model| model["otherRooms"] = json!({(DESTINATION): {"id":DESTINATION,"name":"squad-next","retired":false,"revision":1,"memberIds":[]}}));
    (
        Squad {
            name: "product".into(),
            room_id: ROOM.into(),
        },
        Squad {
            name: "next".into(),
            room_id: DESTINATION.into(),
        },
    )
}

#[test]
fn move_pauses_source_jobs_before_leave_and_keeps_uuid_counter_and_pause_on_readd() {
    let f = Fixture::new();
    let (source, destination) = squads(&f);
    let user = f.actor(USER);
    let job = f.add(&user, WORKER).unwrap().job.job;
    let outcome = assign(
        &f.core,
        Some(&source),
        &destination,
        WORKER,
        true,
        Layout::Crew,
    )
    .unwrap();
    assert!(outcome.complete);
    assert_eq!(outcome.document["pausedTimers"], json!(["c1"]));
    let model = f.model();
    assert!(
        model["rosterWithout"]
            .as_array()
            .unwrap()
            .contains(&json!(WORKER))
    );
    assert!(
        model["otherRooms"][DESTINATION]["memberIds"]
            .as_array()
            .unwrap()
            .contains(&json!(WORKER))
    );
    assert_eq!(model["jobsAtLeave"]["jobs"][0]["state"], "paused");
    let stored = tmt_ops::cron::Store::new(&f.directory.join("ops"))
        .unwrap()
        .read()
        .unwrap();
    let paused = &stored.jobs()[0];
    assert_eq!(paused.room_id, job.room_id);
    assert_eq!(paused.squad, "product");
    assert_eq!(paused.id(), job.id());
    assert_eq!(paused.owner_id, job.owner_id);
    assert!(paused.pause.is_some());
    assert!(
        add(&f.core, &source, Layout::Crew, &[WORKER.into()])
            .unwrap()
            .complete
    );
    let view =
        cron_service::show_job(&f.core, &f.config, &cron_service::JobKey::of(paused), 200).unwrap();
    assert_eq!(view.job.pause, paused.pause);
    assert!(
        f.mutate(&user, &view.job, Mutation::Resume)
            .unwrap()
            .job
            .job
            .pause
            .is_none()
    );
}

#[test]
fn failed_destination_keeps_source_membership_and_the_published_pause() {
    let f = Fixture::new();
    let (source, destination) = squads(&f);
    f.add(&f.actor(USER), WORKER).unwrap();
    f.change_model(|model| model["joinFailure"] = json!("STORAGE_UNAVAILABLE"));
    let outcome = assign(
        &f.core,
        Some(&source),
        &destination,
        WORKER,
        true,
        Layout::Crew,
    )
    .unwrap();
    assert!(!outcome.complete);
    assert!(outcome.document["sourceRemoval"].is_null());
    assert!(
        !f.model()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|call| call["command"][1] == "leave")
    );
    assert!(
        tmt_ops::cron::Store::new(&f.directory.join("ops"))
            .unwrap()
            .read()
            .unwrap()
            .jobs()[0]
            .pause
            .is_some()
    );
}

#[test]
fn add_to_another_squad_preserves_source_membership_and_running_jobs() {
    let f = Fixture::new();
    let (source, destination) = squads(&f);
    let job = f.add(&f.actor(USER), WORKER).unwrap().job.job;
    assert!(
        assign(
            &f.core,
            Some(&source),
            &destination,
            WORKER,
            false,
            Layout::Crew
        )
        .unwrap()
        .complete
    );
    assert_eq!(
        tmt_ops::cron::Store::new(&f.directory.join("ops"))
            .unwrap()
            .read()
            .unwrap()
            .jobs()[0],
        job
    );
    assert!(
        source
            .roster(&f.core)
            .unwrap()
            .iter()
            .any(|member| member.id == WORKER)
    );
}

#[test]
fn reused_destination_name_and_virtual_source_fail_before_pausing_or_joining() {
    let f = Fixture::new();
    let (source, mut destination) = squads(&f);
    let job = f.add(&f.actor(USER), WORKER).unwrap().job.job;
    assert_eq!(
        assign(&f.core, None, &destination, WORKER, true, Layout::Crew)
            .err()
            .unwrap()
            .code,
        "SQUAD_NOT_MEMBER"
    );
    destination.name = "changed".into();
    assert_eq!(
        assign(
            &f.core,
            Some(&source),
            &destination,
            WORKER,
            true,
            Layout::Crew
        )
        .err()
        .unwrap()
        .code,
        "ROOM_REVISION_CONFLICT"
    );
    assert_eq!(
        tmt_ops::cron::Store::new(&f.directory.join("ops"))
            .unwrap()
            .read()
            .unwrap()
            .jobs()[0],
        job
    );
}
