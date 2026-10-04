use super::*;
use crate::cron_service::{Mutation, test_support::*};
use std::{fs, time::Instant};

fn request_calls(model: &Value) -> Vec<&Value> {
    model["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|call| {
            call["request"]["operation"] == "dispatch.create"
                && call["request"]["input"]["kind"] == "request"
        })
        .collect()
}

fn minute_job(f: &Fixture) -> Job {
    let actor = f.actor(LEAD);
    let job = f.add(&actor, WORKER).unwrap().job.job;
    f.mutate(
        &actor,
        &job,
        Mutation::Edit {
            message: None,
            schedule: Some(
                cron::Schedule::parse(
                    cron::ScheduleInput::Every {
                        duration: "1m",
                        from: None,
                    },
                    "UTC",
                    now() - 10_000,
                )
                .unwrap(),
            ),
        },
    )
    .unwrap()
    .job
    .job
}

fn wait_for<T>(mut read: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = read() {
            return value;
        }
        assert!(Instant::now() < deadline, "fixture readiness timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn clock_status_projection_keeps_full_holder_evidence() {
    let holder = cron::Holder {
        pane: Some("%41".into()),
        pid: 123,
        since_ms: 100,
        expires_ms: 30_100,
    };
    assert_eq!(
        status_document(ClockStatus::Running(holder)),
        json!({"state":"running","pane":"%41","pid":123,"sinceMs":100,"expiresMs":30_100})
    );
    assert_eq!(
        status_document(ClockStatus::NoClock),
        json!({"state":"no clock"})
    );
    assert_eq!(
        status_document(ClockStatus::Unknown),
        json!({"state":"unknown"})
    );
    assert!(
        text(
            &json!({"action":"clock","clock":{"state":"no clock"}}),
            Terminal::PLAIN
        )
        .contains("tmt sq cron run")
    );
}

#[test]
fn manual_send_preserves_operator_revision_paused_state_and_exact_message() {
    let f = Fixture::new();
    let lead = f.actor(LEAD);
    let job = f.add(&lead, WORKER).unwrap().job.job;
    let paused = f.mutate(&lead, &job, Mutation::Pause).unwrap().job.job;
    let key = JobKey::of(&paused);
    let before = fs::read(f.directory.join("squad/cron/jobs.json")).unwrap();
    let id = manual_operation().unwrap();
    let accepted = send_now(&f.core, &f.config, &key, &lead, paused.revision, &id).unwrap();
    assert_eq!(accepted["dispatch"]["operationId"], id);
    assert_eq!(
        before,
        fs::read(f.directory.join("squad/cron/jobs.json")).unwrap()
    );
    let model = f.model();
    let calls = request_calls(&model);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["request"]["identity"], LEAD);
    assert!(calls[0]["request"]["originator"].is_null());
    assert_eq!(calls[0]["request"]["input"]["message"], paused.message);
    assert_eq!(
        calls[0]["request"]["input"]["room"],
        json!({"kind":"direct","roomId":ROOM})
    );
    assert_eq!(calls[0]["locked"], false);
    assert_eq!(
        send_now(
            &f.core,
            &f.config,
            &key,
            &lead,
            job.revision,
            &manual_operation().unwrap()
        )
        .unwrap_err()
        .code,
        "SQUAD_CRON_REVISION_CONFLICT"
    );
    assert_eq!(
        send_now(
            &f.core,
            &f.config,
            &key,
            &f.actor(WORKER),
            paused.revision,
            &manual_operation().unwrap()
        )
        .unwrap_err()
        .code,
        "SQUAD_CRON_PERMISSION_DENIED"
    );
    assert_eq!(request_calls(&f.model()).len(), 1);
}

#[test]
fn scheduled_replay_and_lost_response_recover_one_anonymous_acceptance() {
    let f = Fixture::new();
    let job = minute_job(&f);
    let clock = Clock::new(&f.directory).unwrap();
    let mut lease = acquire(&clock, now()).unwrap().unwrap();
    f.change_model(|model| model["loseResponse"] = json!(true));
    let cancellation = Cancellation::default();
    for attempt in 0..3 {
        if attempt == 1 {
            f.change_model(|model| model["loseResponse"] = json!("storage"));
        }
        let result = pass(
            &f.core,
            &f.config,
            &f.directory,
            &mut lease,
            None,
            &cancellation,
        )
        .unwrap();
        assert_eq!(result["accepted"], 1);
        assert_eq!(result["complete"], true);
    }
    lease.release().unwrap();
    let model = f.model();
    assert_eq!(model["wakes"].as_array().unwrap().len(), 1);
    assert_eq!(model["dispatches"].as_object().unwrap().len(), 1);
    assert!(
        model["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|call| call["request"]["operation"] == "dispatch.show")
    );
    for call in request_calls(&model) {
        assert_eq!(call["request"]["originator"], "anonymous");
        assert!(call["request"]["identity"].is_null());
        assert_eq!(call["request"]["input"]["message"], job.message);
        assert_eq!(call["locked"], false);
    }
    assert_eq!(clock.status(now()), ClockStatus::NoClock);
}

#[test]
fn scheduled_admission_rejects_changed_revision_and_current_roster_loss() {
    let f = Fixture::new();
    let job = minute_job(&f);
    let clock = Clock::new(&f.directory).unwrap();
    let mut lease = acquire(&clock, now()).unwrap().unwrap();
    let cancellation = Cancellation::default();
    let mut dispatcher = Scheduled {
        core: &f.core,
        lease: &mut lease,
        cancellation: &cancellation,
        lost: false,
    };
    let changed = f
        .mutate(
            &f.actor(LEAD),
            &job,
            Mutation::Edit {
                message: Some("changed".into()),
                schedule: None,
            },
        )
        .unwrap()
        .job
        .job;
    let id = cron::operation_id(ROOM, &job.id(), 60_000);
    assert_eq!(
        dispatcher.send(&job, &id).unwrap_err().code,
        "SQUAD_CRON_REVISION_CONFLICT"
    );
    f.change_model(|model| model["rosterWithout"] = json!([WORKER]));
    assert_eq!(
        dispatcher.send(&changed, &id).unwrap_err().code,
        "SQUAD_NOT_A_MEMBER"
    );
    assert!(request_calls(&f.model()).is_empty());
    lease.release().unwrap();
}

#[test]
fn expired_lease_stops_scheduled_work_before_dispatch() {
    let f = Fixture::new();
    let job = minute_job(&f);
    let clock = Clock::new(&f.directory).unwrap();
    let mut lease = acquire(&clock, now() - 40_000).unwrap().unwrap();
    let cancellation = Cancellation::default();
    let mut dispatcher = Scheduled {
        core: &f.core,
        lease: &mut lease,
        cancellation: &cancellation,
        lost: false,
    };
    assert_eq!(
        dispatcher
            .send(&job, &cron::operation_id(ROOM, &job.id(), 60_000))
            .unwrap_err()
            .code,
        "SQUAD_CRON_CLOCK_LOST"
    );
    assert!(dispatcher.stopped());
    assert!(request_calls(&f.model()).is_empty());
    lease.release().unwrap();
}

#[test]
fn running_clock_baselines_at_start_and_rejects_a_second_foreground_clock() {
    let f = Fixture::new();
    minute_job(&f); // Its prior slot is within the standalone window.
    let mut first = ClockWorker::spawn(f.core.clone(), f.config.clone(), true);
    let clock = Clock::new(&f.directory).unwrap();
    wait_for(|| matches!(clock.status(now()), ClockStatus::Running(_)).then_some(()));
    // Wait until the first pass has read the jobs, rather than merely its lease.
    wait_for(|| {
        f.model()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|call| call["request"]["operation"] == "identityHooks.pending")
            .then_some(())
    });
    let mut second = ClockWorker::spawn(f.core.clone(), f.config.clone(), true);
    second
        .finished
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert_eq!(second.stop().unwrap_err().code, "SQUAD_CRON_CLOCK_RUNNING");
    assert!(request_calls(&f.model()).is_empty());
    first.stop().unwrap();
    assert_eq!(clock.status(now()), ClockStatus::NoClock);
    // The replacement also starts at now, without replaying the prior slot.
    let mut replacement = ClockWorker::spawn(f.core.clone(), f.config.clone(), true);
    wait_for(|| matches!(clock.status(now()), ClockStatus::Running(_)).then_some(()));
    replacement.stop().unwrap();
    assert!(request_calls(&f.model()).is_empty());
    assert_eq!(clock.status(now()), ClockStatus::NoClock);
}

#[test]
fn worker_shutdown_cancels_and_joins_an_inflight_core_child_then_releases_lease() {
    let f = Fixture::new();
    f.change_model(|model| model["blockOn"] = json!("identityHooks.pending"));
    let mut worker = ClockWorker::spawn(f.core.clone(), f.config.clone(), true);
    let pid: i32 = wait_for(|| {
        fs::read_to_string(f.directory.join("blocked.pid"))
            .ok()
            .and_then(|text| text.parse().ok())
    });
    assert!(matches!(
        Clock::new(&f.directory).unwrap().status(now()),
        ClockStatus::Running(_)
    ));
    let started = Instant::now();
    worker.stop().unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert_eq!(
        Clock::new(&f.directory).unwrap().status(now()),
        ClockStatus::NoClock
    );
    assert!(request_calls(&f.model()).is_empty());
}
