//! Re-exec fixtures keep group ownership independent of Cargo/nextest.
use super::*;
use crate::tests::{Fixture, START_BUDGET, ready};
use nix::{
    errno::Errno,
    sys::{
        signal::{Signal, kill},
        wait::{WaitStatus, waitpid},
    },
    unistd::{Pid, getpgrp},
};
use std::{fs, time::Duration};

#[test]
fn inherited_group_keeps_capture_bounds_and_leaves_cleanup_to_caller() {
    let args = [
        "TMT_INVOKE_INHERITED_FIXTURE=1".into(),
        std::env::current_exe().unwrap().into_os_string(),
        "--exact".into(),
        "inherited_tests::inherited_group_child".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    // The outer default invocation owns this helper group, including on panic.
    // Twice the inner budget lets a failing inner call return its own assertion.
    let output = invoke(
        Request {
            program: Path::new("/usr/bin/env"),
            args: &args,
            input: b"",
            deadline: Instant::now() + 2 * START_BUDGET,
            max_stream_bytes: 16384,
            launch: Default::default(),
        },
        None,
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("caller group survived"));
}

#[test]
#[ignore = "re-exec helper explicitly exercised by the owning test"]
fn inherited_group_child() {
    let Some(mode) = std::env::var_os("TMT_INVOKE_INHERITED_FIXTURE") else {
        return;
    };
    if mode == "probe" {
        println!("group={}", getpgrp());
        return;
    }
    assert_eq!(nix::unistd::getpid(), getpgrp());
    // This process was explicitly launched in a private group. A fixture panic
    // aborts that group too, including a child with already-closed output pipes.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("inherited fixture assertion: {info}");
        let _ = nix::sys::signal::killpg(nix::unistd::getpid(), Signal::SIGKILL);
    }));
    let fixture = Fixture::new(":");
    let call = |script: &str, timeout: Duration, limit, stop| {
        invoke(
            Request {
                program: Path::new("/bin/sh"),
                args: &["-c".into(), script.into()],
                input: b"",
                deadline: Instant::now() + timeout,
                max_stream_bytes: limit,
                launch: LaunchOptions {
                    process_group: ProcessGroup::InheritCaller,
                    ..Default::default()
                },
            },
            stop,
        )
    };
    let output = call("printf raw; printf err >&2", START_BUDGET, 64, None).unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"raw");
    assert_eq!(output.stderr, b"err");
    let probe = invoke(
        Request {
            program: Path::new("/usr/bin/env"),
            args: &[
                "TMT_INVOKE_INHERITED_FIXTURE=probe".into(),
                std::env::current_exe().unwrap().into_os_string(),
                "--exact".into(),
                "inherited_tests::inherited_group_child".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            input: b"",
            deadline: Instant::now() + START_BUDGET,
            max_stream_bytes: 1024,
            launch: LaunchOptions {
                process_group: ProcessGroup::InheritCaller,
                ..Default::default()
            },
        },
        None,
    )
    .unwrap();
    assert!(probe.status.success());
    let text = String::from_utf8(probe.stdout).unwrap();
    let group: i32 = text
        .lines()
        .find_map(|line| line.strip_prefix("group="))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(group, getpgrp().as_raw(), "no setpgid in inherited mode");
    // A limit case has the start budget, so only its limit can end it; the
    // child sleeps far longer than any deadline, so a return proves no wait.
    let deadline = Duration::from_millis(300);
    for (row, (body, timeout, kind)) in [
        ("exec sleep 300", deadline, FailureKind::Deadline),
        (
            "exec >/dev/null 2>&1; exec sleep 300",
            deadline,
            FailureKind::Deadline,
        ),
        (
            "printf 123456789; exec sleep 300",
            START_BUDGET,
            FailureKind::OutputLimit(Stream::Stdout),
        ),
        (
            "printf 123456789 >&2; exec sleep 300",
            START_BUDGET,
            FailureKind::OutputLimit(Stream::Stderr),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let pid_path = fixture.0.join(format!("pid-{row}"));
        let script = format!("echo $$ > '{}'; {body}", pid_path.display());
        let started = Instant::now();
        let error = call(&script, timeout, 8, None).unwrap_err();
        assert_eq!(error.kind, kind);
        assert!(matches!(error.cleanup, Cleanup::CallerOwned));
        assert!(started.elapsed() < timeout + Duration::from_secs(2));
        // A caller-owned child can still be starting after a Deadline return.
        ready(&pid_path);
        let pid = Pid::from_raw(
            fs::read_to_string(&pid_path)
                .unwrap()
                .trim()
                .parse()
                .unwrap(),
        );
        // Both caller and child survive: neither killpg nor direct kill happened.
        assert_eq!(kill(pid, None), Ok(()));
        kill(pid, Signal::SIGKILL).unwrap();
        assert!(matches!(
            waitpid(pid, None).unwrap(),
            WaitStatus::Signaled(_, Signal::SIGKILL, _)
        ));
        assert_eq!(kill(pid, None), Err(Errno::ESRCH));
    }
    let stop = std::sync::atomic::AtomicBool::new(true);
    let error = call("exit 0", Duration::from_secs(1), 64, Some(&stop)).unwrap_err();
    assert!(matches!(error.cleanup, Cleanup::NotStarted));
    assert_eq!(error.kind, FailureKind::Interrupted);
    println!("caller group survived");
}
