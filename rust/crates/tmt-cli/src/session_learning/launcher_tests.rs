//! Managed launch admission with a real model-free npm-shaped process tree.
use super::tests::{launch, wait_file};
use super::*;
use crate::{
    provider_hook_command::{SessionCommit, StartCandidate, commit_session, refused_start},
    run_command::admission::{ForegroundAdmission, admit_foreground},
};
use std::{fs, io::BufRead, os::unix::fs::symlink, path::Path};
use tmt_adapters::{
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
    runtime::{RuntimeCommand, RuntimeRegistry, lifecycle::ForegroundLaunch},
    skill_installation::ProviderEnvironment,
};
use tmt_core::host::HostKind;

/// Re-executed only as the fixture's private native peer; never a provider.
#[test]
fn native_peer() {
    let Some(root) = std::env::var_os("TMT_TEST_NATIVE_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    assert!(root.starts_with(std::env::temp_dir()));
    assert!(tmt_adapters::private_file::directory_is_private(root));
    fs::write(root.join("native.tmp"), std::process::id().to_string()).unwrap();
    fs::rename(root.join("native.tmp"), root.join("native")).unwrap();
    let mut released = String::new();
    std::io::BufReader::new(fs::File::open(root.join("release")).unwrap())
        .read_line(&mut released)
        .unwrap();
    assert_eq!(released, "release\n");
}

#[test]
fn native_session_survives_direct_or_npm_launch_in_both_start_orderings() {
    let mut outcomes = Vec::new();
    for wrapped in [false, true] {
        for early in [true, false] {
            outcomes.push((wrapped, early, retained(wrapped, early)));
        }
    }
    assert!(
        outcomes.iter().all(|(_, _, kept)| *kept),
        "every verified native startup must retain its exact session and launch owner: {outcomes:?}"
    );
}

fn retained(wrapped: bool, early: bool) -> bool {
    let super::tests::Launch {
        directory: _directory,
        paths,
        mut storage,
        identity,
        binding,
        owner,
        driver,
        learning,
        deadline,
    } = launch(HostKind::pty(), "codex");
    let root = paths.global_dir.as_path();
    let native = root.join("native-bin/codex");
    fs::create_dir(native.parent().unwrap()).unwrap();
    tmt_test_support::write_executable(
        &native,
        &fs::read(std::env::current_exe().unwrap()).unwrap(),
        0o700,
    )
    .unwrap();
    let shim = root.join("node_modules/@openai/codex/bin/codex.js");
    fs::create_dir_all(shim.parent().unwrap()).unwrap();
    fs::write(
        shim.parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("package.json"),
        r#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#,
    )
    .unwrap();
    tmt_test_support::write_executable(&shim, br#"#!/usr/bin/env node
const fs = require('node:fs');
const {spawn} = require('node:child_process');
const root = process.env.TMT_TEST_NATIVE_ROOT;
fs.writeFileSync(root + '/launcher.tmp', String(process.pid));
fs.renameSync(root + '/launcher.tmp', root + '/launcher');
const child = spawn(root + '/native-bin/codex', ['--exact', 'session_learning::launcher_tests::native_peer', '--nocapture'], {stdio:'inherit'});
child.on('exit', (code, signal) => { if (signal) process.kill(process.pid, signal); else process.exit(code ?? 1); });
"#, 0o700).unwrap();
    let selected = root.join("bin/codex");
    fs::create_dir(selected.parent().unwrap()).unwrap();
    symlink(&shim, &selected).unwrap();
    let node = ProviderEnvironment::capture()
        .unwrap()
        .find_command("node")
        .expect("Node test prerequisite");
    let search_path = std::env::join_paths([
        node.parent().unwrap(),
        Path::new("/usr/bin"),
        Path::new("/bin"),
    ])
    .unwrap();
    let fifo = root.join("release");
    UnixCommandRunner
        .execute(CommandRequest {
            program: "/usr/bin/mkfifo".as_ref(),
            args: &[fifo.as_os_str().into()],
            input: &[],
            deadline,
            max_output_bytes: 1024,
        })
        .unwrap();
    let command = if wrapped {
        RuntimeCommand {
            executable: selected.into_os_string(),
            args: vec![],
        }
    } else {
        RuntimeCommand {
            executable: native.into_os_string(),
            args: vec![
                "--exact".into(),
                "session_learning::launcher_tests::native_peer".into(),
                "--nocapture".into(),
            ],
        }
    };
    let mut args: Vec<std::ffi::OsString> = vec![
        "-i".into(),
        format!("PATH={}", search_path.to_string_lossy()).into(),
        "TMT_AGENT=1".into(),
        "BROWSER=/usr/bin/true".into(),
        "NODE_DISABLE_COMPILE_CACHE=1".into(),
        format!("HOME={}", root.display()).into(),
        format!("TMPDIR={}", std::env::temp_dir().display()).into(),
        format!("TMT_TEST_NATIVE_ROOT={}", root.display()).into(),
        command.executable.clone(),
    ];
    args.extend(command.args.clone());
    let child = UnixCommandRunner
        .start(CommandRequest {
            program: "/usr/bin/env".as_ref(),
            args: &args,
            input: &[],
            deadline,
            max_output_bytes: 4096,
        })
        .unwrap();
    let native_pid = wait_file(&root.join("native"))
        .trim()
        .parse::<u64>()
        .unwrap();
    let launcher_pid = if wrapped {
        wait_file(&root.join("launcher")).trim().parse().unwrap()
    } else {
        native_pid
    };
    let live = |pid| match observe_runtime_process(&UnixCommandRunner, pid, deadline).unwrap() {
        tmt_adapters::process::runtime::ProcessObservation::Live(value) => value,
        other => panic!("private peer is not live: {other:?}"),
    };
    let native = live(native_pid);
    let launcher = live(launcher_pid);
    let registry = RuntimeRegistry::first_party();
    let lifecycle = registry.lifecycle(&driver).unwrap();
    let event = lifecycle.decode(br#"{"session_id":"01a1906e-fda0-7363-b095-81df96419db9","hook_event_name":"SessionStart","source":"startup","model":"fixture-model"}"#).unwrap();
    let host = HostEvidence::Independent {
        runtime_pid: Some(native_pid as u32),
    };
    let before = Storage::context_by_identity(
        &paths.database,
        &identity.id,
        tmt_adapters::request_runtime::wall_time_ms(),
    )
    .unwrap()
    .unwrap();
    if early {
        assert!(
            learning
                .defer(&before, &native, host, event.as_ref(), lifecycle, deadline)
                .unwrap()
        );
    }
    let runtime = lifecycle.observe_foreground(
        &UnixCommandRunner,
        ForegroundLaunch {
            command: &command,
            owner: &owner,
            pane_pid: binding.pane_pid,
        },
        &tmt_adapters::process::runtime::ProcessObservation::Live(launcher.clone()),
        deadline,
    );
    let admitted = admit_foreground(
        &mut storage,
        ForegroundAdmission {
            binding: &binding,
            owner: Some(&owner),
            child: &runtime,
            driver: Some(&driver),
            resumed: None,
            lifecycle,
            preferences: &before.preferences,
            lease_session: None,
            channel: Some(false),
            can_admit: true,
            plain: None,
        },
    )
    .unwrap();
    if !early && admitted.is_some() {
        let current = Storage::context_by_identity(
            &paths.database,
            &identity.id,
            tmt_adapters::request_runtime::wall_time_ms(),
        )
        .unwrap()
        .unwrap();
        let refused = refused_start(
            StartCandidate {
                stored: &current,
                process: &native,
                previous: tmt_core::binding::session::RuntimeLiveness::Alive,
                host,
            },
            lifecycle,
            &driver,
            event.as_ref(),
            deadline,
        );
        let next = event.propose(
            &current.entry.binding.as_ref().unwrap().session,
            &native,
            tmt_core::binding::session::RuntimeLiveness::Alive,
            host,
            false,
        );
        if refused.is_none()
            && let Some(next) = next
        {
            assert!(
                commit_session(
                    &mut storage,
                    SessionCommit {
                        stored: &current,
                        next: &next,
                        event: event.as_ref(),
                        harness: &driver,
                        mode: &lifecycle.mode(host).unwrap(),
                        fence_preferences: true
                    }
                )
                .unwrap()
            );
        }
    }
    let preferences = storage.session_preferences(&identity.id).unwrap();
    let current = Storage::context_by_identity(
        &paths.database,
        &identity.id,
        tmt_adapters::request_runtime::wall_time_ms(),
    )
    .unwrap()
    .unwrap();
    let state = &current.entry.binding.as_ref().unwrap().session;
    let kept = preferences
        .remembered
        .as_ref()
        .is_some_and(|r| r.provider_session == *event.session())
        && state.key.as_ref().is_some_and(|key| {
            key.incarnation == native && key.provider_session.as_ref() == Some(event.session())
        })
        && state.launch_owner.as_ref() == Some(&owner);
    use std::io::Write;
    let mut release = fs::OpenOptions::new().write(true).open(fifo).unwrap();
    release.write_all(b"release\n").unwrap();
    drop(release);
    child.wait().unwrap();
    for pid in [native_pid, launcher_pid] {
        assert!(matches!(
            observe_runtime_process(&UnixCommandRunner, pid, deadline).unwrap(),
            tmt_adapters::process::runtime::ProcessObservation::Gone
        ));
    }
    storage.close().unwrap();
    eprintln!("NATIVE_LAUNCH wrapped={wrapped} early={early} retained={kept}");
    kept
}
