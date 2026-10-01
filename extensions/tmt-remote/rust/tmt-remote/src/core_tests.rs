use super::*;
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};
#[path = "../tests/support/executable_fixture.rs"]
mod executable_fixture;

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(script: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-613-core-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        executable_fixture::write_executable(
            &fixture.0.join("tmt"),
            &format!("cd '{}'\n{}\n", fixture.0.display(), script),
        )
        .unwrap();
        fixture
    }
    fn client(&self) -> CoreClient {
        CoreClient::at(self.0.join("tmt")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn fixed_public_argv_input_and_core_errors() {
    let fixture = Fixture::new("printf '%s\\n' \"$@\" > argv; cat > input; printf '{\"ok\":true}'");
    let stop = AtomicBool::new(false);
    assert_eq!(
        fixture.client().capabilities(&stop).unwrap(),
        json!({"ok":true})
    );
    assert_eq!(fs::read_to_string(fixture.0.join("argv")).unwrap(), "api\n");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(fixture.0.join("input")).unwrap()).unwrap(),
        json!({"version":1,"operation":"capabilities","input":{}})
    );
    fixture.client().agents(&stop).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.0.join("argv")).unwrap(),
        "list\n--json\n"
    );
    assert!(fs::read(fixture.0.join("input")).unwrap().is_empty());
    let error =
        Fixture::new("printf '{\"error\":{\"code\":\"CORE_CODE\",\"message\":\"kept\"}}'; exit 1");
    assert_eq!(
        error.client().capabilities(&stop).unwrap_err(),
        RemoteError::new("CORE_CODE", "kept")
    );
    for script in ["printf nope", "printf '{}'; exit 2"] {
        assert_eq!(
            Fixture::new(script)
                .client()
                .capabilities(&stop)
                .unwrap_err()
                .code,
            "REMOTE_CORE_UNAVAILABLE"
        );
    }
    assert!(CoreClient::at("relative".into()).is_err());
    assert!(
        CoreClient::at(fixture.0.join("absent"))
            .unwrap()
            .capabilities(&stop)
            .is_err()
    );
}
#[test]
fn output_deadline_interruption_and_reaping() {
    let stop = AtomicBool::new(false);
    for script in ["yes | head -c 4096", "yes >&2 | head -c 4096 >&2"] {
        assert!(
            Fixture::new(script)
                .client()
                .call(&["api"], b"", &stop, Duration::from_secs(2), 64)
                .is_err()
        );
    }
    let fixture = Fixture::new("exec 1>&- 2>&-; sleep 30");
    let started = Instant::now();
    let result = fixture
        .client()
        .call(&["api"], b"", &stop, Duration::from_millis(100), 64);
    let error = result.unwrap_err();
    assert!(
        error.message.contains("timed out"),
        "expected timeout, got {error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    let fixture = Fixture::new("echo $$ > pid; sleep 30 & echo $! > child; wait");
    std::thread::scope(|scope| {
        let root = &fixture.0;
        scope.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !root.join("child").exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            stop.store(true, Ordering::Relaxed);
        });
        assert!(fixture.client().capabilities(&stop).is_err());
    });
    for path in ["pid", "child"] {
        let pid = fs::read_to_string(fixture.0.join(path)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while std::process::Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .output()
            .unwrap()
            .status
            .success()
        {
            assert!(Instant::now() < deadline, "owned {path} leaked");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn invocation_errors_keep_public_mappings_and_cleanup_uncertainty() {
    for phase in [Phase::OpenPipes, Phase::Wait] {
        assert_eq!(
            invocation_error(InvokeError {
                kind: FailureKind::Io(phase),
                cause: Some(std::io::Error::other("fixture")),
                cleanup: Cleanup::Confirmed
            }),
            RemoteError::new("REMOTE_IO", "Remote I/O failed; the door is closed.")
        );
    }
    assert_eq!(
        invocation_error(InvokeError {
            kind: FailureKind::Deadline,
            cause: None,
            cleanup: Cleanup::Unconfirmed(std::io::Error::other("denied"))
        }),
        failure("Core cleanup could not be confirmed; outcome is unknown.")
    );
    assert_eq!(
        invocation_error(InvokeError {
            kind: FailureKind::Interrupted,
            cause: None,
            cleanup: Cleanup::NotStarted
        }),
        failure("Core observation interrupted.")
    );
    assert_eq!(
        invocation_error(InvokeError {
            kind: FailureKind::Io(Phase::Communicate),
            cause: None,
            cleanup: Cleanup::Confirmed
        }),
        failure("Core output could not be read within its bound.")
    );
}
