//! Group-leader preflight and abort exercised in explicitly re-exec'd helpers.
use super::*;
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill, killpg},
    unistd::{Pid, getpgrp, getpid},
};
use std::{fs, process::Command};

fn args(mode: &str, root: &Path) -> Vec<OsString> {
    let mut directory = OsString::from("TMT_SQUAD_INHERITED_ROOT=");
    directory.push(root);
    vec![
        format!("TMT_SQUAD_INHERITED_MODE={mode}").into(),
        directory,
        std::env::current_exe().unwrap().into_os_string(),
        "--exact".into(),
        "runner::inherited_tests::inherited_child".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ]
}

#[test]
fn inherited_context_preserves_leader_preflight_and_started_failure_abort() {
    for mode in [
        "success",
        "preflight",
        "nonleader-parent",
        "deadline",
        "stdout",
        "stderr",
    ] {
        let root =
            std::env::temp_dir().join(format!("squad-inherited-{}-{mode}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let output = tmt_invoke::invoke(
            Request {
                program: Path::new("/usr/bin/env"),
                args: &args(mode, &root),
                input: b"",
                deadline: Instant::now() + Duration::from_secs(10),
                max_stream_bytes: 16384,
                launch: Default::default(),
            },
            None,
        )
        .unwrap();
        let group = Pid::from_raw(
            fs::read_to_string(root.join("group"))
                .unwrap()
                .trim()
                .parse()
                .unwrap(),
        );
        if ["deadline", "stdout", "stderr"].contains(&mode) {
            assert_eq!(
                output.status.signal,
                Some(Signal::SIGKILL as i32),
                "{mode}: {output:?}"
            );
            let pid = Pid::from_raw(
                fs::read_to_string(root.join("pid"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap(),
            );
            let end = Instant::now() + Duration::from_secs(2);
            while killpg(group, None) != Err(Errno::ESRCH) {
                assert!(Instant::now() < end, "owned group survived {mode}");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(kill(pid, None), Err(Errno::ESRCH));
        } else {
            assert!(output.status.success(), "{mode}: {output:?}");
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
#[ignore = "re-exec helper explicitly exercised by the owning test"]
fn inherited_child() {
    let Ok(mode) = std::env::var("TMT_SQUAD_INHERITED_MODE") else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os("TMT_SQUAD_INHERITED_ROOT").unwrap());
    fs::write(root.join("group"), getpgrp().to_string()).unwrap();
    if mode == "nonleader-parent" {
        // Inherit the explicitly owned outer group; don't rely on the test runner.
        assert!(
            Command::new("/usr/bin/env")
                .args(args("nonleader", &root))
                .status()
                .unwrap()
                .success()
        );
        return;
    }
    if mode == "nonleader" {
        assert_ne!(getpid(), getpgrp());
        assert_eq!(
            run_inherited(
                Path::new("/nonexistent/tmt"),
                &[],
                b"",
                Instant::now() + Duration::from_secs(1),
                64,
                EnvironmentPolicy::Inherit,
            )
            .unwrap_err(),
            RunError::Spawn
        );
        return;
    }
    assert_eq!(getpid(), getpgrp());
    if mode == "preflight" {
        assert_eq!(
            run_inherited(
                Path::new("/nonexistent/tmt"),
                &[],
                b"",
                Instant::now() + Duration::from_secs(1),
                64,
                EnvironmentPolicy::Inherit,
            )
            .unwrap_err(),
            RunError::Spawn
        );
        assert_eq!(
            run_inherited(
                Path::new("/bin/sh"),
                &[],
                b"",
                Instant::now(),
                64,
                EnvironmentPolicy::Inherit
            )
            .unwrap_err(),
            RunError::Timeout
        );
        return;
    }
    let script = if mode == "success" {
        "cat; echo diagnostic >&2; exit 3".to_owned()
    } else {
        format!(
            "echo $$ > '{}'; {}; exec sleep 30",
            root.join("pid").display(),
            match mode.as_str() {
                "stdout" => "printf 123456789",
                "stderr" => "printf 123456789 >&2",
                _ => ":",
            }
        )
    };
    let result = run_inherited(
        Path::new("/bin/sh"),
        &["-c".into(), script.into()],
        b"input",
        Instant::now() + Duration::from_millis(300),
        if mode == "success" { 64 } else { 8 },
        EnvironmentPolicy::Inherit,
    );
    if mode == "success" {
        let output = result.unwrap();
        assert!(!output.success);
        assert_eq!(output.stdout, b"input");
    } else {
        panic!("started inherited failure must abort group: {result:?}");
    }
}
