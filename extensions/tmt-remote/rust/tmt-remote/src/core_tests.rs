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
    let root = Fixture::new(
        "printf '%s\\n' \"$@\" > argv; cat > input; printf '{\"dataRoot\":\"/tmp/tmt-root\"}'",
    );
    assert_eq!(
        root.client().storage_root(&stop).unwrap(),
        PathBuf::from("/tmp/tmt-root")
    );
    assert_eq!(fs::read_to_string(root.0.join("argv")).unwrap(), "api\n");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(root.0.join("input")).unwrap()).unwrap(),
        json!({"version":1,"operation":"storage.root","input":{}})
    );
    for reply in ["{}", r#"{"dataRoot":"relative"}"#, r#"{"dataRoot":""}"#] {
        assert_eq!(
            Fixture::new(&format!("printf '{reply}'"))
                .client()
                .storage_root(&stop)
                .unwrap_err()
                .code,
            "REMOTE_CORE_UNAVAILABLE"
        );
    }
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
        RemoteError::new(
            "REMOTE_CORE_UNCERTAIN",
            "Core cleanup could not be confirmed; outcome is unknown."
        )
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

// Re-exec owns the lock in a separate process: SIGKILL bypasses Rust destructors.
#[test]
fn invocation_lease_probe_child() {
    let Some(root) = std::env::var_os("TMT_REMOTE_LEASE_PROBE") else {
        return;
    };
    let root = PathBuf::from(root);
    let layout = crate::state::Layout::open(&root).unwrap();
    let serving = layout.serve_lock().unwrap();
    if std::env::var_os("TMT_REMOTE_LEASE_INHERIT").is_some() {
        serving.retain_for_invocations().unwrap();
    }
    CoreClient::at(root.join("tmt"))
        .unwrap()
        .capabilities(&AtomicBool::new(false))
        .unwrap();
}
#[test]
fn existing_invoke_preserves_explicit_lease_after_owner_crash() {
    use nix::{
        sys::signal::{Signal, kill, killpg},
        sys::stat::Mode,
        unistd::{Pid, mkfifo},
    };
    use std::{
        io::Write,
        process::{Child, Command, Stdio},
    };
    struct Processes {
        owner: Child,
        root: PathBuf,
    }
    impl Drop for Processes {
        fn drop(&mut self) {
            let _ = self.owner.kill();
            let _ = self.owner.wait();
            if let Ok(text) = fs::read_to_string(self.root.join("pid"))
                && let Ok(pid) = text.trim().parse::<i32>()
            {
                let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
            }
        }
    }
    for inherit in [false, true] {
        let fixture = Fixture::new(
            "cat >/dev/null; echo $$ > pid; read release < gate; echo stopped > finished; printf '{\"ok\":true}'",
        );
        mkfifo(&fixture.0.join("gate"), Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
        let mut gate = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(fixture.0.join("gate"))
            .unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "core::tests::invocation_lease_probe_child",
                "--nocapture",
            ])
            .env("TMT_REMOTE_LEASE_PROBE", &fixture.0)
            .env_remove("TMT_REMOTE_LEASE_INHERIT")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if inherit {
            command.env("TMT_REMOTE_LEASE_INHERIT", "1");
        }
        let mut processes = Processes {
            owner: command.spawn().unwrap(),
            root: fixture.0.clone(),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Ok(text) = fs::read_to_string(fixture.0.join("pid"))
                && let Ok(pid) = text.trim().parse::<i32>()
            {
                break pid;
            }
            assert!(
                Instant::now() < deadline,
                "invoked child did not reach barrier"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        processes.owner.kill().unwrap();
        processes.owner.wait().unwrap();
        assert!(
            kill(Pid::from_raw(pid), None).is_ok(),
            "child must still be active"
        );
        let layout = crate::state::Layout::open(&fixture.0).unwrap();
        let restarted = layout.serve_lock();
        if inherit {
            assert!(matches!(restarted, Err(ref error) if error.code == "REMOTE_ALREADY_SERVING"));
        } else {
            assert!(
                restarted.is_ok(),
                "default CLOEXEC lock does not fence the orphan"
            );
        }
        drop(restarted);
        gate.write_all(b"continue\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while kill(Pid::from_raw(pid), None).is_ok() {
            assert!(
                Instant::now() < deadline,
                "invoked child leaked after release"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            fs::read_to_string(fixture.0.join("finished")).unwrap(),
            "stopped\n"
        );
        assert!(layout.serve_lock().is_ok(), "lease must end with the child");
        fs::remove_file(fixture.0.join("pid")).unwrap();
    }
}

#[test]
fn check_resolves_permitted_uuid_by_public_inventory_and_preserves_capture() {
    let id = "11111111-1111-4111-8111-111111111111";
    let stop = AtomicBool::new(false);
    for (name, lines) in [("Current", None), ("Renamed", Some(10))] {
        let directory = json!({"identities":[{"id":id,"name":name,"canonicalName":name.to_lowercase(),"lifetime":"saved"}]});
        let capture = json!({"target":name,"pane":"%9","lines":lines.unwrap_or(20),"output":"actual capture\n","identity":{"name":name,"canonicalName":name.to_lowercase()}});
        let fixture = Fixture::new(&format!(
            "printf '%s\\n' \"$*\" >> calls; cat > input; case \"$1\" in identity) printf '%s' '{directory}';; check) printf '%s' '{capture}';; *) exit 9;; esac"
        ));
        assert_eq!(fixture.client().check(id, lines, &stop).unwrap(), capture);
        let suffix = if lines.is_some() { " --lines 10" } else { "" };
        assert_eq!(
            fs::read_to_string(fixture.0.join("calls")).unwrap(),
            format!("identity list --json\ncheck {name} --json{suffix}\n")
        );
        assert!(fs::read(fixture.0.join("input")).unwrap().is_empty());
    }
}

#[test]
fn check_never_captures_absent_retired_or_ambiguous_identity() {
    let id = "11111111-1111-4111-8111-111111111111";
    let row = json!({"id":id,"name":"Current","canonicalName":"current"});
    // Public inventory omits retired rows. A reused name has a different UUID.
    let reused = json!({"id":"22222222-2222-4222-8222-222222222222","name":"Current","canonicalName":"current"});
    for rows in [
        json!([]),
        json!([reused.clone()]),
        json!([row.clone(), row.clone()]),
        json!([row, reused]),
    ] {
        let directory = json!({"identities":rows});
        let fixture = Fixture::new(&format!(
            "printf '%s\\n' \"$*\" >> calls; cat > input; if [ \"$1\" != identity ]; then touch captured; fi; printf '%s' '{directory}'"
        ));
        assert_eq!(
            fixture
                .client()
                .check(id, None, &AtomicBool::new(false))
                .unwrap_err()
                .code,
            "REMOTE_CORE_UNAVAILABLE"
        );
        assert_eq!(
            fs::read_to_string(fixture.0.join("calls")).unwrap(),
            "identity list --json\n"
        );
        assert!(!fixture.0.join("captured").exists());
    }
}
