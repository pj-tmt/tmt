use super::*;
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new(script: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-invoke-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        let quoted = fixture.0.to_string_lossy().replace('\'', "'\\''");
        fs::write(
            fixture.program(),
            format!("#!/bin/sh\ncd '{quoted}'\n{script}\n"),
        )
        .unwrap();
        fs::set_permissions(fixture.program(), fs::Permissions::from_mode(0o700)).unwrap();
        fixture
    }
    fn program(&self) -> PathBuf {
        self.0.join("child program")
    }
    fn call(
        &self,
        args: &[OsString],
        input: &[u8],
        timeout: Duration,
        limit: usize,
        stop: Option<&AtomicBool>,
    ) -> Result<Output, InvokeError> {
        // Read the script instead of execing an inode a concurrent fork may hold writable.
        let mut shell_args = vec![self.program().into_os_string()];
        shell_args.extend_from_slice(args);
        invoke(
            Request {
                program: Path::new("/bin/sh"),
                args: &shell_args,
                input,
                deadline: Instant::now() + timeout,
                max_stream_bytes: limit,
                launch: Default::default(),
            },
            stop,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn ready(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fs::read_to_string(path).is_ok_and(|value| !value.trim().is_empty()) {
        assert!(
            Instant::now() < deadline,
            "child readiness missing: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn gone(path: &Path) {
    let pid = fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while kill(Pid::from_raw(pid), None) != Err(Errno::ESRCH) {
        assert!(
            Instant::now() < deadline,
            "owned child {pid} survived cleanup"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn bytes_literal_arguments_and_completed_status_are_returned() {
    let fixture = Fixture::new("printf '%s\\n' \"$@\" > args; cat; printf err >&2");
    let args = ["--literal".into(), "space and 'quote'".into()];
    let output = fixture
        .call(&args, b"\0raw\xff", Duration::from_secs(5), 64, None)
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"\0raw\xff");
    assert_eq!(output.stderr, b"err");
    assert_eq!(
        fs::read_to_string(fixture.0.join("args")).unwrap(),
        "--literal\nspace and 'quote'\n"
    );
    let output = Fixture::new("printf arbitrary; printf diagnostic >&2; exit 7")
        .call(&[], b"", Duration::from_secs(5), 64, None)
        .unwrap();
    assert_eq!(output.status.code, Some(7));
    assert!(!output.status.success());
    assert_eq!(output.stdout, b"arbitrary");
    assert_eq!(output.stderr, b"diagnostic");
    let output = Fixture::new("kill -TERM $$")
        .call(&[], b"", Duration::from_secs(5), 64, None)
        .unwrap();
    assert_eq!(output.status.signal, Some(15));
    assert!(!output.status.success());
}

#[test]
fn each_stream_has_its_own_exact_bound_including_zero() {
    let output = Fixture::new("printf 1234; printf 5678 >&2")
        .call(&[], b"", Duration::from_secs(5), 4, None)
        .unwrap();
    assert_eq!(output.stdout, b"1234");
    assert_eq!(output.stderr, b"5678");
    for (script, stream) in [
        ("printf 12345", Stream::Stdout),
        ("printf 12345 >&2", Stream::Stderr),
        ("yes >&2", Stream::Stderr),
    ] {
        let started = Instant::now();
        let error = Fixture::new(script)
            .call(&[], b"", Duration::from_secs(5), 4, None)
            .unwrap_err();
        assert_eq!(error.kind, FailureKind::OutputLimit(stream));
        assert!(matches!(error.cleanup, Cleanup::Confirmed));
        assert!(started.elapsed() < Duration::from_secs(3));
    }
    assert!(
        Fixture::new(":")
            .call(&[], b"", Duration::from_secs(5), 0, None)
            .is_ok()
    );
    assert_eq!(
        Fixture::new("printf x")
            .call(&[], b"", Duration::from_secs(5), 0, None)
            .unwrap_err()
            .kind,
        FailureKind::OutputLimit(Stream::Stdout)
    );
}

#[test]
fn stdin_and_both_output_pipes_make_progress_under_pressure() {
    let input = vec![b'i'; 256 * 1024];
    let fixture = Fixture::new("head -c 262144 /dev/zero; head -c 262144 /dev/zero >&2; cat");
    // Quiet/loaded maxima: 13.7/16.7 ms; ~20 ms at 1 ms pulses with 12 CPU workers.
    // subprocess 1.2.1 retains write/read progress across TimedOut; no livelock found.
    // One 5 s Deadline miss under heavy full-workspace load remains unexplained.
    // 30 s matches real-process success budgets in tmt-adapters host/external/tests.rs.
    let output = fixture
        .call(&[], &input, Duration::from_secs(30), 512 * 1024, None)
        .unwrap();
    assert_eq!(&output.stdout[..256 * 1024], vec![0; 256 * 1024]);
    assert_eq!(&output.stdout[256 * 1024..], input);
    assert_eq!(output.stderr, vec![0; 256 * 1024]);
}

#[test]
fn blocked_input_and_closed_output_do_not_escape_the_deadline() {
    // Allow the same startup budget as other real-process fixtures under parallel load.
    let input = vec![b'x'; 1024 * 1024];
    for (script, input) in [
        ("echo $$ > pid; exec sleep 30", input.as_slice()),
        (
            "echo $$ > pid; exec 1>&- 2>&-; exec sleep 30",
            b"".as_slice(),
        ),
    ] {
        let fixture = Fixture::new(script);
        let started = Instant::now();
        let error = fixture
            .call(&[], input, Duration::from_secs(5), 64, None)
            .unwrap_err();
        ready(&fixture.0.join("pid"));
        assert_eq!(error.kind, FailureKind::Deadline);
        assert!(matches!(error.cleanup, Cleanup::Confirmed));
        assert!(started.elapsed() < Duration::from_secs(7));
        gone(&fixture.0.join("pid"));
    }
}

#[test]
fn cancellation_prevents_start_or_kills_a_ready_owned_group() {
    let stop = AtomicBool::new(true);
    let fixture = Fixture::new("echo started > marker");
    let error = fixture
        .call(&[], b"", Duration::from_secs(5), 64, Some(&stop))
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Interrupted);
    assert!(matches!(error.cleanup, Cleanup::NotStarted));
    assert!(!fixture.0.join("marker").exists());
    stop.store(false, Ordering::Relaxed);
    let fixture = Fixture::new("echo $$ > pid; sleep 30 & echo $! > descendant; wait");
    std::thread::scope(|scope| {
        scope.spawn(|| {
            ready(&fixture.0.join("descendant"));
            stop.store(true, Ordering::Relaxed);
        });
        let error = fixture
            .call(&[], b"", Duration::from_secs(5), 64, Some(&stop))
            .unwrap_err();
        assert_eq!(error.kind, FailureKind::Interrupted);
        assert!(matches!(error.cleanup, Cleanup::Confirmed));
    });
    gone(&fixture.0.join("pid"));
    gone(&fixture.0.join("descendant"));
}

#[test]
fn a_descendant_holding_pipes_is_terminated_before_leader_reap() {
    let fixture = Fixture::new("echo $$ > pid; sleep 30 & echo $! > descendant; exit 0");
    let error = fixture
        .call(&[], b"", Duration::from_secs(5), 64, None)
        .unwrap_err();
    ready(&fixture.0.join("descendant"));
    assert_eq!(error.kind, FailureKind::Deadline);
    assert!(matches!(error.cleanup, Cleanup::Confirmed));
    gone(&fixture.0.join("pid"));
    gone(&fixture.0.join("descendant"));
}

#[test]
fn failed_spawn_and_expired_request_never_leave_a_child() {
    let fixture = Fixture::new("echo started > marker");
    let error = invoke(
        Request {
            program: Path::new("/bin/sh"),
            args: &[fixture.program().into_os_string()],
            input: b"",
            deadline: Instant::now(),
            max_stream_bytes: 64,
            launch: Default::default(),
        },
        None,
    )
    .unwrap_err();
    assert_eq!(error.kind, FailureKind::Deadline);
    assert!(matches!(error.cleanup, Cleanup::NotStarted));
    assert!(!fixture.0.join("marker").exists());
    let error = invoke(
        Request {
            program: &fixture.0.join("missing"),
            args: &[],
            input: b"",
            deadline: Instant::now() + Duration::from_secs(5),
            max_stream_bytes: 64,
            launch: Default::default(),
        },
        None,
    )
    .unwrap_err();
    assert_eq!(error.kind, FailureKind::Spawn);
    assert!(error.cause.is_some());
    assert!(matches!(error.cleanup, Cleanup::NotStarted));
}

#[test]
fn path_search_preserves_order_relative_candidates_and_permissions() {
    let first = Fixture::new(":");
    let second = Fixture::new(":");
    let search = std::env::join_paths([&first.0, &second.0]).unwrap();
    assert_eq!(
        find_executable(OsStr::new("child program"), &search),
        Some(first.program())
    );
    fs::set_permissions(first.program(), fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        find_executable(OsStr::new("child program"), &search),
        Some(second.program())
    );
    assert!(!is_executable(&first.0));
    assert!(!is_executable(&first.program()));
    assert!(find_executable(OsStr::new("missing"), &search).is_none());
    let relative = Path::new("../../../Cargo.toml");
    assert!(!is_executable(relative));
    let exe = std::env::current_exe().unwrap();
    let child = Command::new(exe)
        .args([
            "--ignored",
            "--exact",
            "tests::discovery_child",
            "--nocapture",
        ])
        .current_dir(&second.0)
        .env("INVOKE_EXPECT", "relative")
        .env_remove("TMT_EXECUTABLE")
        .output()
        .unwrap();
    assert!(
        child.status.success()
            && String::from_utf8_lossy(&child.stdout).contains("discovery fixture checked"),
        "{}",
        String::from_utf8_lossy(&child.stdout)
    );
}

#[test]
fn discovery_uses_only_the_supplied_environment_path() {
    for (supplied, expected) in [
        (None, "missing"),
        (Some("relative"), "invalid"),
        (Some("/absent/invoking-program"), "absolute"),
    ] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--ignored",
                "--exact",
                "tests::discovery_child",
                "--nocapture",
            ])
            .env("INVOKE_EXPECT", expected)
            .env_remove("TMT_EXECUTABLE");
        if let Some(path) = supplied {
            child.env("TMT_EXECUTABLE", path);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("discovery fixture checked"),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

// A fresh test process isolates environment and cwd without unsafe mutation.
#[test]
#[ignore]
fn discovery_child() {
    match std::env::var("INVOKE_EXPECT").unwrap().as_str() {
        "missing" => assert_eq!(invoking_tmt(), Err(DiscoveryError::Missing)),
        "invalid" => assert_eq!(
            invoking_tmt(),
            Err(DiscoveryError::NotAbsolute("relative".into()))
        ),
        "absolute" => assert_eq!(
            invoking_tmt().unwrap(),
            Path::new("/absent/invoking-program")
        ),
        "relative" => assert_eq!(
            find_executable(OsStr::new("child program"), OsStr::new(".")),
            Some(PathBuf::from("./child program"))
        ),
        _ => panic!("unknown discovery fixture"),
    }
    println!("discovery fixture checked");
}

#[test]
fn child_environment_policy_does_not_mutate_the_caller() {
    use std::os::unix::ffi::OsStringExt;
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "tests::environment_policy_child",
            "--nocapture",
        ])
        .env_clear()
        .env("INVOKE_KEEP", "space and 'literal'")
        .env("INVOKE_SECRET", "excluded-test-secret")
        .env("INVOKE_BYTES", OsString::from_vec(vec![b'x', 0xff]))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("environment fixture checked"));
}

#[test]
#[ignore = "isolated environment fixture launched by the parent test"]
fn environment_policy_child() {
    use std::os::unix::ffi::OsStrExt;
    let snapshot = || {
        let mut vars: Vec<_> = std::env::vars_os().collect();
        vars.sort();
        vars
    };
    let before = snapshot();
    let capture = |environment| {
        let output = invoke(
            Request {
                program: Path::new("/usr/bin/env"),
                args: &[],
                input: b"",
                deadline: Instant::now() + Duration::from_secs(5),
                max_stream_bytes: 4096,
                launch: LaunchOptions { environment },
            },
            None,
        )
        .unwrap();
        assert!(output.status.success() && output.stderr.is_empty());
        output.stdout
    };
    let inherited = capture(EnvironmentPolicy::default());
    assert!(
        inherited
            .windows(b"INVOKE_SECRET=excluded-test-secret".len())
            .any(|w| w == b"INVOKE_SECRET=excluded-test-secret")
    );
    assert!(capture(EnvironmentPolicy::ClearAllowlist(&[])).is_empty());
    let names = [
        "INVOKE_KEEP".into(),
        "INVOKE_BYTES".into(),
        "INVOKE_MISSING".into(),
        "INVOKE_KEEP".into(),
    ];
    let allowed = capture(EnvironmentPolicy::ClearAllowlist(&names));
    let mut lines: Vec<_> = allowed
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            b"INVOKE_BYTES=x\xff".as_slice(),
            b"INVOKE_KEEP=space and 'literal'".as_slice()
        ]
    );
    assert_eq!(
        std::env::var_os("INVOKE_BYTES").unwrap().as_bytes(),
        b"x\xff"
    );
    assert_eq!(snapshot(), before);
    println!("environment fixture checked");
}
