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
        .contains("tmt ops sq cron run")
    );
}

#[test]
fn clock_text_uses_relative_age_without_changing_exact_json_evidence() {
    let since_ms = 100_000;
    let document = json!({"action":"clock","clock":status_document(ClockStatus::Running(cron::Holder {
        pane: Some("%41".into()), pid: 123, since_ms, expires_ms: 400_000,
    }))});
    let unchanged = document.clone();
    let text = text_at(&document, Terminal::PLAIN, since_ms + 180_000);
    assert!(
        text.lines()
            .any(|line| line.trim_start().starts_with("since") && line.ends_with("3m ago")),
        "{text}"
    );
    assert!(!text.contains("1970-"));
    assert!(text_at(&document, Terminal::PLAIN, since_ms - 1).contains("just now"));
    assert_eq!(document, unchanged);
    assert_eq!(document["clock"]["sinceMs"], since_ms);
}

#[test]
fn manual_send_preserves_operator_revision_paused_state_and_exact_message() {
    let f = Fixture::new();
    let lead = f.actor(LEAD);
    let job = f.add(&lead, WORKER).unwrap().job.job;
    let paused = f.mutate(&lead, &job, Mutation::Pause).unwrap().job.job;
    let key = JobKey::of(&paused);
    let before = fs::read(f.directory.join("ops/cron/jobs.json")).unwrap();
    let id = manual_operation().unwrap();
    let accepted = send_now(&f.core, &f.config, &key, &lead, paused.revision, &id).unwrap();
    assert_eq!(accepted["dispatch"]["operationId"], id);
    assert_eq!(
        before,
        fs::read(f.directory.join("ops/cron/jobs.json")).unwrap()
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
    let clock = Clock::new(&f.directory.join("ops")).unwrap();
    let mut lease = acquire(&f.core, &clock, now()).unwrap().unwrap();
    f.change_model(|model| model["loseResponse"] = json!(true));
    let cancellation = Cancellation::default();
    for attempt in 0..3 {
        if attempt == 1 {
            f.change_model(|model| model["loseResponse"] = json!("storage"));
        }
        let result = pass(
            &f.core,
            &f.config,
            &f.directory.join("ops"),
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
    let clock = Clock::new(&f.directory.join("ops")).unwrap();
    let mut lease = acquire(&f.core, &clock, now()).unwrap().unwrap();
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
    let clock = Clock::new(&f.directory.join("ops")).unwrap();
    let mut lease = acquire(&f.core, &clock, now() - 40_000).unwrap().unwrap();
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
    let clock = Clock::new(&f.directory.join("ops")).unwrap();
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
        Clock::new(&f.directory.join("ops")).unwrap().status(now()),
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
        Clock::new(&f.directory.join("ops")).unwrap().status(now()),
        ClockStatus::NoClock
    );
    assert!(request_calls(&f.model()).is_empty());
}

#[test]
fn standby_shutdown_cancels_an_inflight_root_read_without_releasing_another_holder() {
    let f = Fixture::new();
    let root = service::root(&f.core).unwrap();
    let clock = Clock::new(&root).unwrap();
    let lease = clock
        .acquire(now(), 123, Some("%41".into()))
        .unwrap()
        .unwrap();
    let held = fs::read(root.join("cron/clock.json")).unwrap();
    f.change_model(|model| model["blockOn"] = json!("storage.root"));
    let mut worker = ClockWorker::spawn(f.core.clone(), f.config.clone(), false);
    let pid: i32 = wait_for(|| {
        fs::read_to_string(f.directory.join("blocked.pid"))
            .ok()
            .and_then(|text| text.parse().ok())
    });
    let started = Instant::now();
    let stopped = worker.stop();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert_eq!(fs::read(root.join("cron/clock.json")).unwrap(), held);
    assert!(request_calls(&f.model()).is_empty());
    lease.release().unwrap();
    assert_eq!(
        stopped.unwrap(),
        json!({"action":"run","warnings":[],"complete":true})
    );
    assert_eq!(worker.stop().unwrap(), json!({"action":"run"}));
}

#[test]
fn stop_preserves_a_completed_core_error_even_with_the_cancellation_message() {
    let f = Fixture::new();
    let selected = f.directory.join("error-tmt");
    crate::test_support::write_ready_executable(
        &selected,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"error\":{\"code\":\"SQUAD_CORE_UNAVAILABLE\",\"message\":\"The board load was superseded.\"}}'\nexit 1\n",
    );
    let mut worker = ClockWorker::spawn(Core::at(selected), f.config.clone(), false);
    worker
        .finished
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        worker.stop().unwrap_err().to_json(),
        json!({"error":{"code":"SQUAD_CORE_UNAVAILABLE","message":"The board load was superseded."}})
    );
    assert!(!f.directory.join("ops/cron/clock.json").exists());
    assert!(request_calls(&f.model()).is_empty());
}

#[test]
fn deferred_ui_workers_never_take_the_old_clock_and_migrate_without_stopping() {
    let mut f = Fixture::new();
    let actor = f.actor(LEAD);
    f.add(&actor, WORKER).unwrap();
    let original = fs::read(f.directory.join("ops/cron/jobs.json")).unwrap();
    fs::rename(f.directory.join("ops"), f.directory.join("squad")).unwrap();
    fs::rename(f.directory.join("ops.toml"), f.directory.join("squad.toml")).unwrap();
    for name in [".ops-paths-v1", ".ops-paths-cutover-v1"] {
        fs::remove_file(f.directory.join(name)).unwrap();
    }
    let old_clock = Clock::new(&f.directory.join("squad")).unwrap();
    let old_lease = old_clock
        .acquire(now(), 123, Some("%41".into()))
        .unwrap()
        .unwrap();
    let held = fs::read(f.directory.join("squad/cron/clock.json")).unwrap();
    f.core = Core::at(f.core.executable().into());
    f.config = Config::load(&f.core).unwrap();
    let calls = || {
        f.model()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|call| call["request"]["operation"] == "storage.root")
            .count()
    };
    let before = calls();
    let mut first = ClockWorker::spawn(f.core.clone(), f.config.clone(), false);
    let mut second = ClockWorker::spawn(f.core.clone(), f.config.clone(), false);
    wait_for(|| (calls() >= before + 2).then_some(()));
    assert_eq!(
        fs::read(f.directory.join("squad/cron/clock.json")).unwrap(),
        held
    );
    assert!(!f.directory.join("ops").exists());
    assert!(request_calls(&f.model()).is_empty());
    // Simulate the former board removing its unchanged lease. Migration probes
    // can hold this lock; Lease::release consumes the lease even on BUSY, so the
    // fixture waits for the guard before removing its own evidence.
    let old_directory = f.directory.join("squad/cron");
    let guard = wait_for(|| {
        let file = fs::File::open(old_directory.join("clock.lock")).unwrap();
        match nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock) {
            Ok(guard) => Some(guard),
            Err((_, nix::errno::Errno::EAGAIN)) => None,
            Err((_, error)) => panic!("fixture clock lock: {error}"),
        }
    });
    assert_eq!(fs::read(old_directory.join("clock.json")).unwrap(), held);
    fs::remove_file(old_directory.join("clock.json")).unwrap();
    fs::File::open(&old_directory).unwrap().sync_all().unwrap();
    drop(guard);
    drop(old_lease);
    wait_for(|| f.directory.join(".ops-paths-v1").exists().then_some(()));
    wait_for(|| {
        // A contended status is Unknown, so keep observing until Running.
        Clock::new(&f.directory.join("ops"))
            .ok()
            .is_some_and(|clock| matches!(clock.status(now()), ClockStatus::Running(_)))
            .then_some(())
    });
    assert!(!crate::migration::paths(&f.core, None).unwrap().legacy);
    assert_eq!(
        Config::load(&f.core).unwrap().path(),
        f.directory.join("ops.toml")
    );
    assert_eq!(
        fs::read(f.directory.join("ops/cron/jobs.json")).unwrap(),
        original
    );
    assert!(!f.directory.join("squad").exists());
    assert!(first.finished.try_recv().is_err());
    assert!(second.finished.try_recv().is_err());
    first.stop().unwrap();
    second.stop().unwrap();
    assert!(!f.directory.join("ops/cron/clock.json").exists());
}

#[test]
fn stopping_a_deferred_ui_worker_is_interruptible_and_preserves_the_old_holder() {
    let f = Fixture::new();
    crate::migration::paths(&f.core, None).unwrap();
    fs::rename(f.directory.join("ops.toml"), f.directory.join("squad.toml")).unwrap();
    for name in [".ops-paths-v1", ".ops-paths-cutover-v1"] {
        fs::remove_file(f.directory.join(name)).unwrap();
    }
    let old = Clock::new(&f.directory.join("squad")).unwrap();
    let lease = old.acquire(now(), 123, None).unwrap().unwrap();
    let core = Core::at(f.core.executable().into());
    let config = Config::load(&core).unwrap();
    let mut worker = ClockWorker::spawn(core, config, false);
    let started = Instant::now();
    worker.stop().unwrap();
    assert!(
        started.elapsed() < EVERY,
        "disconnect must wake the retry wait"
    );
    assert!(matches!(old.status(now()), ClockStatus::Running(holder) if holder.pid == 123));
    assert!(!f.directory.join("ops").exists());
    lease.release().unwrap();
}

#[test]
fn deferred_worker_shutdown_cancels_and_joins_an_inflight_migration_read() {
    let f = Fixture::new();
    crate::migration::paths(&f.core, None).unwrap();
    fs::rename(f.directory.join("ops.toml"), f.directory.join("squad.toml")).unwrap();
    for name in [".ops-paths-v1", ".ops-paths-cutover-v1"] {
        fs::remove_file(f.directory.join(name)).unwrap();
    }
    let old = Clock::new(&f.directory.join("squad")).unwrap();
    let lease = old.acquire(now(), 123, None).unwrap().unwrap();
    let core = Core::at(f.core.executable().into());
    let config = Config::load(&core).unwrap();
    f.change_model(|model| model["blockOn"] = json!("storage.root"));
    let mut worker = ClockWorker::spawn(core, config, false);
    let pid: i32 = wait_for(|| {
        fs::read_to_string(f.directory.join("blocked.pid"))
            .ok()
            .and_then(|text| text.parse().ok())
    });
    let started = Instant::now();
    worker.stop().unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert!(matches!(old.status(now()), ClockStatus::Running(holder) if holder.pid == 123));
    assert!(!f.directory.join("ops").exists());
    lease.release().unwrap();
}

/// Digest stands in only for its public process/lock lifetime, never delivery semantics.
fn digest_core(f: &Fixture, mode: &str) -> Core {
    fs::write(f.directory.join("digest-mode"), mode).unwrap();
    fs::write(
        f.directory.join("digest.py"),
        r#"
import pathlib,sys,fcntl,os,signal
p=pathlib.Path(__file__).parent
assert sys.argv[1:]==['digest','tick'], sys.argv
with open(p/'digest-calls','a') as calls: calls.write('tick\n')
lock=open(p/'digest.lock','a+')
try: fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
except BlockingIOError: sys.exit(0)
with open(p/'digest-acquired','a') as calls: calls.write('acquired\n')
mode=(p/'digest-mode').read_text()
if mode=='fail':
 print('unavailable extension',file=sys.stderr); sys.exit(3)
if mode=='block':
 (p/'digest.pid').write_text(str(os.getpid()))
 signal.pause()
"#,
    )
    .unwrap();
    let executable = f.directory.join("with-digest");
    crate::test_support::write_ready_executable(
        &executable,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = digest ]; then exec python3 '{}' \"$@\"; fi\nexec '{}' \"$@\"\n",
            f.directory.join("digest.py").display(),
            f.core.executable().display()
        ),
    );
    Core::at(executable)
}

fn digest_calls(f: &Fixture, name: &str) -> usize {
    fs::read_to_string(f.directory.join(name)).map_or(0, |text| text.lines().count())
}

#[test]
fn digest_minute_gate_has_no_same_minute_retry_catchup_or_rollback_replay() {
    let f = Fixture::new();
    let core = digest_core(&f, "success");
    let mut ticks = DigestTicks::new(&core, false);
    for (time, calls) in [
        (61_000, 1),
        (119_999, 1),
        (120_000, 2),
        (600_000, 3),
        (60_000, 3),
        (600_001, 3),
    ] {
        ticks.advance(time);
        wait_for(|| {
            ticks.reap();
            ticks.children.is_empty().then_some(())
        });
        assert_eq!(digest_calls(&f, "digest-calls"), calls);
    }
    assert_eq!(ticks.skipped, 0);
    assert_eq!(ticks.last_reason, None);
    drop(ticks);
    // A new clock run can attempt the current minute; Digest owns lock admission.
    let mut restarted = DigestTicks::new(&core, false);
    restarted.advance(600_002);
    wait_for(|| {
        restarted.reap();
        restarted.children.is_empty().then_some(())
    });
    assert_eq!(digest_calls(&f, "digest-calls"), 4);
}

#[test]
fn digest_nonzero_and_missing_process_are_quiet_one_attempt_per_minute() {
    let f = Fixture::new();
    let core = digest_core(&f, "fail");
    let mut ticks = DigestTicks::new(&core, false);
    ticks.advance(0);
    wait_for(|| {
        ticks.reap();
        ticks.children.is_empty().then_some(())
    });
    assert_eq!(ticks.skipped, 1);
    assert_eq!(ticks.last_reason, Some("nonzero exit"));
    ticks.advance(59_999);
    assert_eq!(digest_calls(&f, "digest-calls"), 1);
    ticks.advance(60_000);
    wait_for(|| {
        ticks.reap();
        ticks.children.is_empty().then_some(())
    });
    assert_eq!(ticks.skipped, 2);
    assert_eq!(digest_calls(&f, "digest-calls"), 2);
    let missing = Core::at(f.directory.join("absent-tmt"));
    let mut ticks = DigestTicks::new(&missing, false);
    ticks.advance(0);
    wait_for(|| {
        ticks.reap();
        ticks.children.is_empty().then_some(())
    });
    assert_eq!(ticks.last_reason, Some("unavailable"));
    ticks.advance(1);
    assert_eq!(ticks.skipped, 1);
    assert!(ticks.children.is_empty());
}

#[test]
fn digest_overlap_is_admitted_by_extension_and_shutdown_reaps_the_waiting_child() {
    use nix::{sys::signal::kill, unistd::Pid};
    let f = Fixture::new();
    let core = digest_core(&f, "block");
    let mut ticks = DigestTicks::new(&core, false);
    ticks.advance(0);
    let pid: i32 = wait_for(|| {
        fs::read_to_string(f.directory.join("digest.pid"))
            .ok()
            .and_then(|s| s.parse().ok())
    });
    ticks.advance(60_000); // Does not wait for the first child's in-minute deadline.
    wait_for(|| {
        ticks.reap();
        (digest_calls(&f, "digest-calls") == 2 && ticks.children.len() == 1).then_some(())
    });
    assert_eq!(digest_calls(&f, "digest-acquired"), 1);
    assert_eq!(ticks.skipped, 0);
    drop(ticks);
    assert_eq!(
        kill(Pid::from_raw(pid), None).unwrap_err(),
        nix::errno::Errno::ESRCH
    );
}

#[test]
fn board_and_explicit_clocks_run_digest_without_refresh_and_stop_before_lease_release() {
    use nix::{sys::signal::kill, unistd::Pid};
    for foreground in [false, true] {
        let f = Fixture::new();
        let core = digest_core(&f, "block");
        // No board, refresh worker, session or renderer is started by this fixture.
        let mut worker = ClockWorker::spawn(core.clone(), f.config.clone(), foreground);
        let pid: i32 = wait_for(|| {
            fs::read_to_string(f.directory.join("digest.pid"))
                .ok()
                .and_then(|s| s.parse().ok())
        });
        let clock = Clock::new(&f.directory.join("ops")).unwrap();
        let first_expiry = match clock.status(now()) {
            ClockStatus::Running(holder) => holder.expires_ms,
            other => panic!("{other:?}"),
        };
        wait_for(|| {
            matches!(clock.status(now()), ClockStatus::Running(holder) if holder.expires_ms > first_expiry).then_some(())
        });
        assert!(
            f.model()["calls"]
                .as_array()
                .unwrap()
                .iter()
                .any(|call| call["request"]["operation"] == "identityHooks.pending"),
            "cron pass progresses while digest waits"
        );
        worker.stop().unwrap();
        assert_eq!(
            kill(Pid::from_raw(pid), None).unwrap_err(),
            nix::errno::Errno::ESRCH
        );
        assert_eq!(clock.status(now()), ClockStatus::NoClock);
        fs::remove_file(f.directory.join("digest.pid")).unwrap();
        let mut restarted = ClockWorker::spawn(core, f.config.clone(), foreground);
        wait_for(|| {
            (digest_calls(&f, "digest-acquired") == 2 && f.directory.join("digest.pid").exists())
                .then_some(())
        });
        restarted.stop().unwrap();
        assert_eq!(clock.status(now()), ClockStatus::NoClock);
    }
}
