use super::*;
use crate::cron_service::{Mutation, test_support::*};
use crate::test_support::stored_jobs;
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

/// Polls observable state; a timeout panics at the waiting call, not in this helper.
#[track_caller]
fn wait_for<T>(read: impl FnMut() -> Option<T>) -> T {
    wait_for_within(Duration::from_secs(10), read, String::new)
}

/// `wait_for` with its own budget; a timeout reports `context`, what the fixture saw.
#[track_caller]
fn wait_for_within<T>(
    budget: Duration,
    mut read: impl FnMut() -> Option<T>,
    context: impl Fn() -> String,
) -> T {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(value) = read() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "fixture readiness timed out {}",
            context()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits until a worker has published its lease. `Clock::status` takes the clock lock, so
/// polling it while a foreground worker makes its single acquire attempt can make that
/// attempt `SQUAD_CRON_CLOCK_BUSY` and end the worker; the lease file needs no lock.
#[track_caller]
fn wait_for_lease(f: &Fixture) {
    let lease = f.directory.join("ops/cron/clock.json");
    wait_for(|| lease.exists().then_some(()));
}

#[test]
fn clock_status_projection_keeps_full_holder_evidence() {
    let holder = cron::Holder {
        pane: Some("%41".into()),
        pid: 123,
        since_ms: 100,
        expires_ms: 30_100,
        build: None,
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
        pane: Some("%41".into()), pid: 123, since_ms, expires_ms: 400_000, build: None, }))});
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
    let before = stored_jobs(&f.directory);
    let id = manual_operation().unwrap();
    let accepted = send_now(&f.core, &key, &lead, paused.revision, &id).unwrap();
    assert_eq!(accepted["dispatch"]["operationId"], id);
    assert_eq!(before, stored_jobs(&f.directory));
    let model = f.model();
    let calls = request_calls(&model);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["request"]["identity"], LEAD);
    assert!(calls[0]["request"]["originator"].is_null());
    assert_eq!(calls[0]["request"]["input"]["message"], paused.message);
    assert_eq!(calls[0]["request"]["input"]["origin"], "automation");
    assert_eq!(
        calls[0]["request"]["input"]["room"],
        json!({"kind":"direct","roomId":ROOM})
    );
    assert_eq!(calls[0]["locked"], false);
    assert_eq!(
        send_now(
            &f.core,
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
fn an_older_core_that_refuses_origin_is_asked_again_once_without_it() {
    let f = Fixture::new();
    let lead = f.actor(LEAD);
    let job = f.add(&lead, WORKER).unwrap().job.job;
    f.change_model(|model| model["rejectOrigin"] = json!(true));
    let id = manual_operation().unwrap();
    let key = JobKey::of(&job);
    let accepted = send_now(&f.core, &key, &lead, job.revision, &id).unwrap();
    assert_eq!(accepted["dispatch"]["operationId"], id);
    let model = f.model();
    let calls = request_calls(&model);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["request"]["input"]["origin"], "automation");
    assert!(calls[1]["request"]["input"].get("origin").is_none());
    assert_eq!(calls[0]["request"]["input"]["operationId"], id);
    assert_eq!(calls[1]["request"]["input"]["operationId"], id);
    // The refused first attempt stored nothing: one request was accepted.
    assert_eq!(model["wakes"].as_array().unwrap().len(), 1);
}

#[test]
fn scheduled_replay_and_lost_response_recover_one_anonymous_acceptance() {
    let f = Fixture::new();
    let job = minute_job(&f);
    let clock = Clock::new(&f.directory.join("ops")).unwrap();
    let mut lease = acquire(&f.core, &clock, now(), false).unwrap().unwrap();
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
    let mut lease = acquire(&f.core, &clock, now(), false).unwrap().unwrap();
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
    let mut lease = acquire(&f.core, &clock, now() - 40_000, false)
        .unwrap()
        .unwrap();
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
    wait_for_lease(&f);
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
    let refused = second.stop().unwrap_err();
    assert_eq!(refused.code, "SQUAD_CRON_CLOCK_RUNNING");
    // The refusal names the clock that holds the lease: this very process runs both workers.
    assert!(
        refused
            .message
            .contains(&format!("process {}", std::process::id())),
        "{}",
        refused.message
    );
    assert!(request_calls(&f.model()).is_empty());
    first.stop().unwrap();
    assert_eq!(clock.status(now()), ClockStatus::NoClock);
    // The replacement also starts at now, without replaying the prior slot.
    let mut replacement = ClockWorker::spawn(f.core.clone(), f.config.clone(), true);
    wait_for_lease(&f);
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

/// A board's own readers poll `Clock::status`, which takes the clock lock for the length of a
/// read. Quitting must not turn that moment into a failed exit.
#[test]
fn shutdown_waits_out_a_brief_clock_lock_holder_before_releasing_the_lease() {
    let f = Fixture::new();
    let mut worker = ClockWorker::spawn(f.core.clone(), f.config.clone(), false);
    wait_for_lease(&f);
    let lock_path = f.directory.join("ops/cron/clock.lock");
    let guard = wait_for(|| {
        let file = fs::File::open(&lock_path).unwrap();
        match nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock) {
            Ok(guard) => Some(guard),
            Err((_, nix::errno::Errno::EAGAIN)) => None,
            Err((_, error)) => panic!("fixture clock lock: {error}"),
        }
    });
    let holder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(guard);
    });
    assert_eq!(
        worker.stop().unwrap(),
        json!({"action":"run","warnings":[],"complete":true})
    );
    holder.join().unwrap();
    assert!(!f.directory.join("ops/cron/clock.json").exists());
}

#[test]
fn standby_shutdown_cancels_an_inflight_root_read_without_releasing_another_holder() {
    let f = Fixture::new();
    let root = service::root(&f.core).unwrap();
    let clock = Clock::new(&root).unwrap();
    let lease = clock
        .acquire(now(), 123, Some("%41".into()), "0.1.0")
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
    let original = stored_jobs(&f.directory);
    fs::rename(f.directory.join("ops"), f.directory.join("squad")).unwrap();
    fs::rename(f.directory.join("ops.toml"), f.directory.join("squad.toml")).unwrap();
    for name in [".ops-paths-v1", ".ops-paths-cutover-v1"] {
        fs::remove_file(f.directory.join(name)).unwrap();
    }
    let old_clock = Clock::new(&f.directory.join("squad")).unwrap();
    let old_lease = old_clock
        .acquire(now(), 123, Some("%41".into()), "0.1.0")
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
    // The workers retry once a second and the migration moves the legacy tree with directory
    // syncs, so a loaded host needs more than an ordinary fixture wait (#4806). A timeout says
    // what each worker did instead of leaving only the bare marker missing.
    wait_for_within(
        Duration::from_secs(60),
        || f.directory.join(".ops-paths-v1").exists().then_some(()),
        || {
            let mut names = fs::read_dir(&f.directory)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            names.sort();
            format!(
                "after 60 s: first worker {:?}, second worker {:?}, data directory {names:?}",
                first.finished.try_recv(),
                second.finished.try_recv()
            )
        },
    );
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
    assert_eq!(stored_jobs(&f.directory), original);
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
    let lease = old.acquire(now(), 123, None, "0.1.0").unwrap().unwrap();
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
    let lease = old.acquire(now(), 123, None, "0.1.0").unwrap().unwrap();
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
    let mut ticks = MinuteJobs::new(&core, false);
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
    let mut restarted = MinuteJobs::new(&core, false);
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
    let mut ticks = MinuteJobs::new(&core, false);
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
    let mut ticks = MinuteJobs::new(&missing, false);
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
    let mut ticks = MinuteJobs::new(&core, false);
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
        // `stop` has joined every tick, so these counts and the pid file are final. A
        // shutdown that overlaps a minute rollover cancels two ticks at once: the
        // second can take the digest lock between the first one dying and its own
        // cancellation, so one session leaves one or two acquisitions. Readiness is
        // therefore a new acquisition after the restart, not a fixed total.
        let before = digest_calls(&f, "digest-acquired");
        fs::remove_file(f.directory.join("digest.pid")).unwrap();
        let mut restarted = ClockWorker::spawn(core, f.config.clone(), foreground);
        let restarted_pid: i32 = wait_for(|| {
            if digest_calls(&f, "digest-acquired") <= before {
                return None;
            }
            fs::read_to_string(f.directory.join("digest.pid"))
                .ok()
                .and_then(|s| s.parse().ok())
        });
        restarted.stop().unwrap();
        assert_eq!(
            kill(Pid::from_raw(restarted_pid), None).unwrap_err(),
            nix::errno::Errno::ESRCH
        );
        assert_eq!(clock.status(now()), ClockStatus::NoClock);
    }
}

/// The public API is the only workspace seam: no tmux or Core files are opened.
fn workspace_core(f: &Fixture, mode: &str) -> Core {
    fs::write(f.directory.join("workspace-mode"), mode).unwrap();
    let script = f.directory.join("workspace.py");
    fs::write(&script, r#"
import json,os,pathlib,signal,sys
p=pathlib.Path(__file__).parent
if sys.argv[1:]==['digest','tick']:
 with open(p/'workspace-digest-calls','a') as log: log.write('tick\n')
 sys.exit(0)
if sys.argv[1:]==['api']:
 raw=sys.stdin.read(); request=json.loads(raw); op=request['operation']
 if op in ['capabilities','workspace.capture']:
  assert request=={'version':1,'operation':op,'input':{}}, request
  with open(p/'workspace-calls','a') as log: log.write(op+'\n')
  mode=(p/'workspace-mode').read_text()
  if (mode=='block' and op=='workspace.capture') or (mode=='discovery-block' and op=='capabilities'):
   signal.signal(signal.SIGUSR1, lambda *_: None)
   (p/'workspace.pid').write_text(str(os.getpid()))
   signal.pause()
  print('workspace diagnostic must stay captured',file=sys.stderr)
  if mode=='discovery-failure' or (mode=='failure' and op=='workspace.capture'):
   print(json.dumps({'error':{'code':'API_INPUT_INVALID','message':'workspace failure must stay captured'}})); sys.exit(1)
  if op=='capabilities':
   print(json.dumps({'operations':[] if mode=='old' else ['workspace.capture']} if mode!='malformed' else {}))
  else: print(json.dumps({'attempted':1,'published':1,'skipped':0}))
  sys.exit(0)
 # Non-workspace cron requests still use the existing independent public-port fixture.
 import subprocess
 sys.exit(subprocess.run([str(p/'tmt'),*sys.argv[1:]],input=raw,text=True).returncode)
os.execv(str(p/'tmt'),[str(p/'tmt'),*sys.argv[1:]])
"#).unwrap();
    let executable = f.directory.join("with-workspace");
    crate::test_support::write_ready_executable(
        &executable,
        &format!("#!/bin/sh\nexec python3 '{}' \"$@\"\n", script.display()),
    );
    Core::at(executable)
}

fn workspace_calls(f: &Fixture) -> Vec<String> {
    fs::read_to_string(f.directory.join("workspace-calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn workspace_capture_shares_minute_cadence_and_discovers_once_per_session() {
    let f = Fixture::new();
    let core = workspace_core(&f, "success");
    let mut ticks = MinuteJobs::new(&core, false);
    for (time, captures) in [
        (61_000, 1),
        (119_999, 1),
        (120_000, 2),
        (600_000, 3),
        (60_000, 3),
    ] {
        ticks.advance(time);
        wait_for(|| {
            ticks.reap();
            let calls = workspace_calls(&f);
            (calls.iter().filter(|op| *op == "workspace.capture").count() == captures
                && ticks.capture.is_none()
                && ticks.children.is_empty())
            .then_some(())
        });
        assert_eq!(
            workspace_calls(&f)
                .iter()
                .filter(|op| *op == "capabilities")
                .count(),
            1
        );
    }
    drop(ticks);
    let mut restarted = MinuteJobs::new(&core, false);
    restarted.advance(600_001);
    wait_for(|| {
        restarted.reap();
        let calls = workspace_calls(&f);
        assert_ne!(
            restarted.capture_supported,
            Some(false),
            "restart discovery failed: {calls:?}"
        );
        (calls.len() == 6 && restarted.capture.is_none() && restarted.children.is_empty())
            .then_some(())
    });
    assert_eq!(
        workspace_calls(&f),
        [
            "capabilities",
            "workspace.capture",
            "workspace.capture",
            "workspace.capture",
            "capabilities",
            "workspace.capture"
        ]
    );
}

#[test]
fn workspace_capture_suppresses_overlap_without_holding_up_digest_or_shutdown() {
    use nix::{sys::signal::kill, unistd::Pid};
    let f = Fixture::new();
    let core = workspace_core(&f, "block");
    let mut ticks = MinuteJobs::new(&core, false);
    ticks.advance(0);
    let pid: i32 = wait_for(|| {
        fs::read_to_string(f.directory.join("workspace.pid"))
            .ok()
            .and_then(|s| s.parse().ok())
    });
    ticks.advance(60_000);
    wait_for(|| {
        ticks.reap();
        (digest_calls(&f, "workspace-digest-calls") == 2).then_some(())
    });
    assert_eq!(workspace_calls(&f), ["capabilities", "workspace.capture"]);
    let start = Instant::now();
    drop(ticks);
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(
        kill(Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
}

#[test]
fn workspace_discovery_and_capture_share_the_clock_shutdown_owner() {
    use nix::{sys::signal::kill, unistd::Pid};
    for foreground in [false, true] {
        for mode in ["discovery-block", "block"] {
            let f = Fixture::new();
            let core = workspace_core(&f, mode);
            let mut worker = ClockWorker::spawn(core, f.config.clone(), foreground);
            let pid: i32 = wait_for(|| {
                fs::read_to_string(f.directory.join("workspace.pid"))
                    .ok()
                    .and_then(|s| s.parse().ok())
            });
            let start = Instant::now();
            let result = worker.stop().unwrap();
            assert!(start.elapsed() < Duration::from_secs(2));
            assert_eq!(result["warnings"], json!([]));
            assert_eq!(
                kill(Pid::from_raw(pid), None),
                Err(nix::errno::Errno::ESRCH)
            );
            assert_eq!(
                Clock::new(&f.directory.join("ops")).unwrap().status(now()),
                ClockStatus::NoClock
            );
        }
    }
}

#[test]
fn workspace_old_or_malformed_answer_is_a_session_local_noop() {
    for mode in ["old", "malformed"] {
        let f = Fixture::new();
        let core = workspace_core(&f, mode);
        let mut ticks = MinuteJobs::new(&core, false);
        for time in [0, 60_000, 120_000] {
            ticks.advance(time);
            wait_for(|| {
                ticks.reap();
                (ticks.capture.is_none() && ticks.children.is_empty()).then_some(())
            });
        }
        assert_eq!(ticks.capture_supported, Some(false));
        assert_eq!(workspace_calls(&f), ["capabilities"]);
    }
}

#[test]
fn workspace_discovery_errors_retry_next_minute_and_cache_recovery() {
    for mode in ["discovery-failure", "missing"] {
        let f = Fixture::new();
        let core = if mode == "missing" {
            Core::at(f.directory.join("with-workspace"))
        } else {
            workspace_core(&f, mode)
        };
        let mut ticks = MinuteJobs::new(&core, false);
        ticks.advance(0);
        wait_for(|| {
            ticks.reap();
            (ticks.capture.is_none() && ticks.children.is_empty()).then_some(())
        });
        assert_eq!(ticks.capture_supported, None, "{mode}");
        let failed_calls = workspace_calls(&f);
        // Publish a working executable at the same path after the failed admission.
        workspace_core(&f, "success");
        ticks.advance(1);
        assert!(ticks.capture.is_none());
        assert_eq!(workspace_calls(&f), failed_calls);
        for time in [60_000, 120_000] {
            ticks.advance(time);
            wait_for(|| {
                ticks.reap();
                (ticks.capture.is_none() && ticks.children.is_empty()).then_some(())
            });
        }
        let mut expected = failed_calls;
        expected
            .extend(["capabilities", "workspace.capture", "workspace.capture"].map(str::to_owned));
        assert_eq!(workspace_calls(&f), expected);
        assert_eq!(ticks.capture_supported, Some(true));
    }
}

#[test]
fn workspace_capture_failure_does_not_change_digest_or_retry_the_same_minute() {
    let f = Fixture::new();
    let core = workspace_core(&f, "failure");
    // Foreground is intentional: workspace failure must not become a Digest diagnostic.
    let mut ticks = MinuteJobs::new(&core, true);
    for time in [0, 1, 60_000] {
        ticks.advance(time);
        wait_for(|| {
            ticks.reap();
            (ticks.capture.is_none() && ticks.children.is_empty()).then_some(())
        });
    }
    assert_eq!(
        workspace_calls(&f),
        ["capabilities", "workspace.capture", "workspace.capture"]
    );
    assert_eq!(digest_calls(&f, "workspace-digest-calls"), 2);
    assert_eq!(ticks.skipped, 0);
    assert_eq!(ticks.last_reason, None);
}

#[test]
fn workspace_discovery_timeout_reaps_then_retries_next_minute() {
    use nix::{sys::signal::kill, unistd::Pid};
    let f = Fixture::new();
    let core = workspace_core(&f, "discovery-block");
    let mut ticks = MinuteJobs::new(&core, false);
    let start = Instant::now();
    ticks.advance(0);
    wait_for(|| {
        ticks.reap();
        (ticks.capture.is_none() && ticks.children.is_empty()).then_some(())
    });
    assert!(start.elapsed() < Duration::from_secs(7));
    assert_eq!(workspace_calls(&f), ["capabilities"]);
    let pid: i32 = fs::read_to_string(f.directory.join("workspace.pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        kill(Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert_eq!(ticks.capture_supported, None);
    fs::write(f.directory.join("workspace-mode"), "success").unwrap();
    ticks.advance(1);
    assert!(ticks.capture.is_none());
    assert_eq!(workspace_calls(&f), ["capabilities"]);
    ticks.advance(60_000);
    wait_for(|| {
        ticks.reap();
        (ticks.capture.is_none() && ticks.children.is_empty()).then_some(())
    });
    assert_eq!(ticks.capture_supported, Some(true));
    assert_eq!(
        workspace_calls(&f),
        ["capabilities", "capabilities", "workspace.capture"]
    );
}

#[test]
fn workspace_optional_job_emits_no_stdout_or_stderr() {
    const PROBE: &str = "TMT_TEST_WORKSPACE_QUIET_PROBE";
    if let Some(directory) = std::env::var_os(PROBE) {
        let directory = std::path::PathBuf::from(directory);
        let core = Core::at(directory.join("with-workspace"));
        // Include noisy success and failure streams, not only an absent operation.
        for mode in [
            "success",
            "failure",
            "old",
            "malformed",
            "discovery-failure",
        ] {
            fs::write(directory.join("workspace-mode"), mode).unwrap();
            let mut ticks = MinuteJobs::new(&core, true);
            ticks.advance(0);
            wait_for(|| {
                ticks.reap();
                (ticks.capture.is_none() && ticks.children.is_empty()).then_some(())
            });
            assert_eq!(ticks.skipped, 0);
        }
        return;
    }
    // Publish and prewarm executables through the existing fixture owner before
    // measuring the subprocess's output; macOS assessment is not capture work.
    let f = Fixture::new();
    let _core = workspace_core(&f, "success");
    let executable = std::env::current_exe().unwrap();
    let args = [
        "--exact".into(),
        "cron_clock::tests::workspace_optional_job_emits_no_stdout_or_stderr".into(),
        "--nocapture".into(),
    ];
    let environment = [(PROBE.into(), f.directory.as_os_str().to_owned())];
    let output = tmt_invoke::invoke(
        tmt_invoke::Request {
            program: &executable,
            args: &args,
            input: b"",
            deadline: Instant::now() + Duration::from_secs(15),
            max_stream_bytes: 64 * 1024,
            launch: tmt_invoke::LaunchOptions {
                environment: tmt_invoke::EnvironmentPolicy::InheritWith(&environment),
                ..Default::default()
            },
        },
        None,
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(
        lines.len(),
        3,
        "only the test harness writes stdout: {stdout}"
    );
    assert_eq!(lines[0], "running 1 test");
    assert_eq!(
        lines[1],
        "test cron_clock::tests::workspace_optional_job_emits_no_stdout_or_stderr ... ok"
    );
    assert!(
        lines[2].starts_with("test result: ok. 1 passed;"),
        "{stdout}"
    );
}
