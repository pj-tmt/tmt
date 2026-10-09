//! The launcher against a real worker process: readiness, failure, silence, a worker that
//! ignores the cancel, an invalid record and a signal that races the handoff.
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tmt_extension_serve::{Handshake, Launch, StartupError, Timing, launch};

struct Dir(PathBuf);
impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tmt-serve-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn pid(&self) -> i32 {
        fs::read_to_string(self.0.join("pid"))
            .unwrap()
            .parse()
            .unwrap()
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn worker() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/fixture_worker")
}
fn handshake(startup: Duration, cleanup_wait: Duration) -> Handshake {
    Handshake {
        unconfirmed: StartupError::new("TEST_UNCONFIRMED", "Startup could not be confirmed."),
        cancelled: StartupError::new("TEST_CANCELLED", "Startup was cancelled."),
        timing: Timing {
            startup,
            cleanup_wait,
            record_bytes: 4096,
        },
    }
}
fn run(
    mode: &str,
    dir: &Dir,
    handshake: &Handshake,
    stop: &AtomicBool,
    validate: impl FnOnce(Value) -> Result<Value, StartupError>,
) -> Result<Value, StartupError> {
    let args: Vec<OsString> = vec![mode.into(), dir.0.clone().into()];
    launch(
        &Launch {
            program: &worker(),
            args: &args,
            handshake,
        },
        stop,
        validate,
    )
}
/// Whether the process still exists; a reaped child is gone.
fn alive(pid: i32) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
fn eventually(what: &str, mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < until, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
const SECOND: Duration = Duration::from_secs(1);
fn ok(value: Value) -> Result<Value, StartupError> {
    Ok(value)
}

#[test]
fn a_ready_worker_serves_only_after_the_accept_and_the_launcher_returns_its_record() {
    let dir = Dir::new("ready");
    let ready = run(
        "ready",
        &dir,
        &handshake(10 * SECOND, 10 * SECOND),
        &AtomicBool::new(false),
        ok,
    )
    .unwrap();
    assert_eq!(ready, json!({"state":"ready"}));
    eventually("the worker began serving", || {
        dir.0.join("serving").exists()
    });
}

#[test]
fn a_failure_the_worker_reports_arrives_typed_with_its_cleanup_confirmed_and_no_child_left() {
    let dir = Dir::new("fail");
    let error = run(
        "fail",
        &dir,
        &handshake(10 * SECOND, 10 * SECOND),
        &AtomicBool::new(false),
        ok,
    )
    .unwrap_err();
    assert_eq!(error.code, "FIXTURE_FAILED");
    assert_eq!(error.hint.as_deref(), Some("Try again."));
    assert!(
        !alive(dir.pid()),
        "the launcher reaped the exact worker it started"
    );
    assert!(!dir.0.join("serving").exists());
}

#[test]
fn a_worker_that_never_reports_is_cancelled_within_the_deadline_and_reaped() {
    let dir = Dir::new("silent");
    let started = Instant::now();
    let error = run(
        "silent",
        &dir,
        &handshake(Duration::from_millis(300), 10 * SECOND),
        &AtomicBool::new(false),
        ok,
    )
    .unwrap_err();
    assert_eq!(error.code, "TEST_UNCONFIRMED");
    assert!(started.elapsed() < 8 * SECOND);
    assert!(!alive(dir.pid()));
}

#[test]
fn a_worker_that_ignores_the_cancel_is_killed_as_our_own_child_and_reported_unconfirmed() {
    let dir = Dir::new("stubborn");
    let error = run(
        "stubborn",
        &dir,
        &handshake(Duration::from_millis(300), Duration::from_millis(300)),
        &AtomicBool::new(false),
        ok,
    )
    .unwrap_err();
    assert_eq!(error.code, "TEST_UNCONFIRMED");
    assert!(!alive(dir.pid()), "no worker outlives an uncertain startup");
}

#[test]
fn an_invalid_ready_record_cancels_the_worker_and_is_the_products_error() {
    let dir = Dir::new("invalid");
    let error = run(
        "ready",
        &dir,
        &handshake(10 * SECOND, 10 * SECOND),
        &AtomicBool::new(false),
        |_| {
            Err(StartupError::new(
                "TEST_INVALID",
                "Not the product's readiness.",
            ))
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "TEST_INVALID");
    assert!(!alive(dir.pid()));
    assert!(!dir.0.join("serving").exists(), "serving never started");
}

#[test]
fn a_signal_before_the_accept_cancels_the_startup_and_nothing_serves() {
    let dir = Dir::new("signal");
    let stop = AtomicBool::new(false);
    let error = run(
        "ready",
        &dir,
        &handshake(10 * SECOND, 10 * SECOND),
        &stop,
        |value| {
            // The shutdown signal lands after Ready and before the launcher accepts.
            stop.store(true, Ordering::SeqCst);
            Ok(value)
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "TEST_CANCELLED");
    assert!(!alive(dir.pid()));
    assert!(!dir.0.join("serving").exists());
}
