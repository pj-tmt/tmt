use super::{
    publication::Layout,
    receipt::Receipt,
    test_support::{artifact, state},
};
use crate::{
    interrupt::Interrupt,
    test_support::{TestChild, TestDirectory},
};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use std::{
    env, fs, io,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const FIXTURE_TEST: &str = "native_install::interrupt_tests::subprocess_fixture";
const FIXTURE_ROOT: &str = "TMT_NATIVE_INSTALL_INTERRUPT_ROOT";
const FIXTURE_READY: &str = "TMT_NATIVE_INSTALL_INTERRUPT_READY";
const FIXTURE_EVENT: &str = "TMT_NATIVE_INSTALL_INTERRUPT_EVENT";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
const EXIT_TIMEOUT: Duration = Duration::from_secs(4);

struct Fixture {
    // Drop the child before the directory removes its readiness/event paths.
    child: TestChild,
    _directory: TestDirectory,
    ready: PathBuf,
    event: PathBuf,
}

impl Fixture {
    fn start() -> Self {
        let directory = TestDirectory::new();
        let ready = directory.path.join("ready");
        let event = directory.path.join("event");
        let executable = env::current_exe().expect("locate native-install test binary");
        let child = Command::new(executable)
            .args(["--exact", FIXTURE_TEST, "--nocapture", "--test-threads=1"])
            .env(FIXTURE_ROOT, &directory.path)
            .env(FIXTURE_READY, &ready)
            .env(FIXTURE_EVENT, &event)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn task-owned native-install interrupt fixture");
        Self {
            _directory: directory,
            ready,
            event,
            child: TestChild::new(child),
        }
    }
}

fn fixture_pid(fixture: &Fixture) -> Pid {
    Pid::from_raw(i32::try_from(fixture.child.child.id()).expect("fixture PID fits in pid_t"))
}

#[test]
fn sigint_cleans_staged_native_publication_and_allows_retry() {
    let mut fixture = Fixture::start();
    fixture
        .child
        .wait_for_content(&fixture.ready.clone(), "staged", STARTUP_TIMEOUT);
    kill(fixture_pid(&fixture), Signal::SIGINT).expect("signal task-owned fixture child");

    let status = fixture.child.wait_for_exit(EXIT_TIMEOUT);
    assert!(
        status.success(),
        "native-install interrupt fixture failed: {status}"
    );
    assert_eq!(
        fs::read_to_string(&fixture.event).unwrap(),
        "interrupted-clean-retry"
    );
}

#[test]
fn subprocess_fixture() {
    let Some(root) = env::var_os(FIXTURE_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    let ready = PathBuf::from(env::var_os(FIXTURE_READY).expect("fixture readiness path"));
    let event = PathBuf::from(env::var_os(FIXTURE_EVENT).expect("fixture event path"));
    let prefix = root.join("prefix");
    let layout = Layout::open(&prefix).expect("open task-owned native-install layout");
    let first_artifact = artifact("1.2.3", b"old synthetic tmt payload\n");
    let first_receipt = Receipt::new(&first_artifact, state("1.2.3", None));
    let mut initial_checkpoint = || Ok(());
    layout
        .publish(
            &first_artifact,
            &first_receipt,
            None,
            None,
            &mut initial_checkpoint,
        )
        .expect("publish initial synthetic release");

    let next_artifact = artifact("1.2.4", b"next synthetic tmt payload\n");
    let next_receipt = Receipt::new(&next_artifact, state("1.2.4", None));
    let next_release = layout
        .root
        .join("releases")
        .join(next_receipt.id.to_string());
    let next_pointer = layout.root.join(format!(".current-{}", next_receipt.id));
    let old_target =
        fs::read_link(layout.root.join("current")).expect("read initial current pointer");
    let old_payload = fs::read(layout.root.join(&old_target).join("tmt"))
        .expect("read initial synthetic payload");

    let interrupt = Interrupt::install().expect("install task-owned interrupt guard");
    let lock = crate::file_lock::exclusive(&layout.root.join("install.lock"))
        .expect("acquire native-install lock");
    let mut calls = 0;
    let mut checkpoint = || {
        if calls == 1 {
            assert!(
                next_release.is_dir(),
                "publication checkpoint must observe its staged release"
            );
            fs::write(&ready, b"staged").expect("publish staged readiness");
            interrupt
                .wait_until(Instant::now() + Duration::from_secs(3))
                .expect("wait for SIGINT in publication checkpoint");
            if !interrupt.is_interrupted() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "publication interrupt was not delivered",
                ));
            }
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "publication interrupted by SIGINT",
            ));
        }
        calls += 1;
        Ok(())
    };
    let error = layout
        .publish(
            &next_artifact,
            &next_receipt,
            Some(first_receipt.id),
            None,
            &mut checkpoint,
        )
        .expect_err("SIGINT must stop publication before activation");
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    drop(lock);
    drop(interrupt);

    assert_eq!(
        fs::read_link(layout.root.join("current")).unwrap(),
        old_target
    );
    assert_eq!(
        fs::read(layout.root.join(&old_target).join("tmt")).unwrap(),
        old_payload
    );
    assert!(fs::symlink_metadata(next_release).is_err());
    assert!(fs::symlink_metadata(next_pointer).is_err());

    let retry_lock = crate::file_lock::exclusive(&layout.root.join("install.lock"))
        .expect("publication lock must be released after interruption");
    let mut retry_checkpoint = || Ok(());
    layout
        .publish(
            &next_artifact,
            &next_receipt,
            Some(first_receipt.id),
            None,
            &mut retry_checkpoint,
        )
        .expect("retry publication after SIGINT");
    drop(retry_lock);
    assert_eq!(
        fs::read_link(layout.root.join("current")).unwrap(),
        PathBuf::from(format!("releases/{}", next_receipt.id))
    );
    fs::write(&event, b"interrupted-clean-retry").expect("publish fixture result");
}
