use super::*;
use tmt_core::{endpoint::ProcessIncarnation, workspace::*};

pub(super) fn sample() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        captured_at_ms: 1,
        server: WorkspaceServer {
            socket: "/tmp/example/socket".into(),
            process: ProcessIncarnation::new(10, "native-start").unwrap(),
            id: None,
        },
        sessions: vec![WorkspaceSession {
            id: "$1".into(),
            name: "workspace".into(),
            windows: vec![WindowLink {
                index: 0,
                window: "@1".into(),
                active: true,
            }],
        }],
        windows: vec![WorkspaceWindow {
            id: "@1".into(),
            name: "shell".into(),
            layout: "abc,80x24,0,0,1".into(),
            visible_layout: "abc,80x24,0,0,1".into(),
            width: 80,
            height: 24,
            active_pane: "%1".into(),
        }],
        panes: vec![WorkspacePane {
            id: "%1".into(),
            window: "@1".into(),
            index: 0,
            left: 0,
            top: 0,
            width: 80,
            height: 24,
            cwd: "/tmp".into(),
            identity: None,
            command: None,
        }],
    }
}

#[test]
fn codec_preserves_literal_external_arguments_and_linked_windows() {
    let mut snapshot = sample();
    let command = ExternalCommand {
        argv: vec![
            "tmt".into(),
            "example".into(),
            "ui".into(),
            "--tabs=a,b".into(),
            "$(never executed)".into(),
        ],
        owner: ProcessIncarnation::new(20, "owner-start").unwrap(),
    };
    snapshot.panes[0].command = Some(command.clone());
    snapshot.sessions.push(WorkspaceSession {
        id: "$2".into(),
        name: "linked".into(),
        windows: snapshot.sessions[0].windows.clone(),
    });
    assert_eq!(decode(&encode(&snapshot).unwrap()).unwrap(), snapshot);
    let marker = encode_command(&command).unwrap();
    assert!(!marker.contains(','));
    assert_eq!(decode_command(&marker), Some(command));
}

#[derive(Clone)]
struct DispatchRunner(std::rc::Rc<crate::scripted_runner::ScriptedRunner>);

impl crate::process::CommandRunner for DispatchRunner {
    fn execute(
        &self,
        request: crate::process::CommandRequest<'_>,
    ) -> Result<crate::process::CommandOutput, crate::process::CommandError> {
        self.0.execute(request)
    }
}

fn dispatch_context(
    directory: &std::path::Path,
    runner: DispatchRunner,
) -> (
    ConfigPaths,
    CallerEnvironment,
    Host<DispatchRunner>,
    DispatchRunner,
) {
    let caller = CallerEnvironment {
        tmux: Some("/tmp/dispatch.sock,10,0".into()),
        pane: Some("%7".into()),
        process_id: 42,
        driver_env: Default::default(),
    };
    let host = Host::for_caller_with(&caller, runner.clone());
    (paths(directory), caller, host, runner)
}

#[test]
fn non_foreground_dispatch_does_no_discovery_subprocess_or_file_work() {
    let runner = DispatchRunner(Default::default());
    let marker = prepare_dispatch("example", &[], 42, |tty, pid| {
        assert_eq!((tty, pid), ("/dev/tty", 42));
        false
    }, || -> Option<(ConfigPaths, CallerEnvironment, Host<DispatchRunner>, DispatchRunner)> {
        panic!("non-foreground dispatch must return before discovering config or host")
    });
    assert!(marker.is_none());
    assert!(runner.0.calls.borrow().is_empty());
}

#[test]
fn eligible_dispatch_only_reads_selected_tty_and_owner_then_writes_marker() {
    let directory = crate::test_support::TestDirectory::new();
    let runner = DispatchRunner(Default::default());
    runner.0.push_output(b"/dev/pts/7\n".to_vec(), vec![]);
    runner
        .0
        .push_output(b"42 Sun Sep 27 10:00:00 2026 S\n".to_vec(), vec![]);
    runner.0.push_output(vec![], vec![]);
    let before = Instant::now();
    let marker = prepare_dispatch(
        "example",
        &["literal $(data)".into()],
        42,
        |_, _| true,
        || Some(dispatch_context(&directory.path, runner.clone())),
    )
    .unwrap();
    let calls = runner.0.calls.borrow();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[0].args,
        [
            "-S",
            "/tmp/dispatch.sock",
            "display-message",
            "-p",
            "-t",
            "%7",
            "#{pane_tty}"
        ]
    );
    assert_eq!(
        calls[1].args[4], "42",
        "one starts read, only for the caller"
    );
    assert_eq!(
        &calls[2].args[..7],
        [
            "-S",
            "/tmp/dispatch.sock",
            "set-option",
            "-p",
            "-t",
            "%7",
            "@tmt.workspace-command"
        ]
    );
    assert!(calls.iter().all(|call| call.deadline == calls[0].deadline));
    assert!(calls[0].deadline >= before);
    assert!(calls[0].deadline.saturating_duration_since(Instant::now()) <= CAPTURE_BUDGET);
    assert!(
        calls
            .iter()
            .all(|call| !call.args.iter().any(|arg| arg == "list-panes"))
    );
    assert!(
        !paths(&directory.path)
            .workspace_directory("/tmp/dispatch.sock")
            .exists(),
        "no snapshot publication before exec"
    );
    assert!(
        !paths(&directory.path).database.exists(),
        "no storage projection"
    );
    let command = decode_command(&marker.document).unwrap();
    assert_eq!(command.argv, ["tmt", "example", "literal $(data)"]);
    assert_eq!(command.owner.pid(), 42);
}

#[test]
fn foreign_or_unknown_selected_tty_refuses_before_starts_or_marker_write() {
    let directory = crate::test_support::TestDirectory::new();
    for tty in ["/dev/pts/8\n", "malformed\n", "/dev/pts/7\nextra\n"] {
        let runner = DispatchRunner(Default::default());
        runner.0.push_output(tty.as_bytes().to_vec(), vec![]);
        assert!(
            prepare_dispatch(
                "example",
                &[],
                42,
                |tty, _| tty == "/dev/tty",
                || Some(dispatch_context(&directory.path, runner.clone()))
            )
            .is_none()
        );
        assert_eq!(runner.0.calls.borrow().len(), 1);
    }
}

#[test]
fn codec_refuses_unknown_versions_and_inconsistent_structure() {
    let mut value: serde_json::Value = serde_json::from_slice(&encode(&sample()).unwrap()).unwrap();
    value["version"] = serde_json::json!(2);
    assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
    value["version"] = serde_json::json!(1);
    value["panes"][0]["window"] = serde_json::json!("@missing");
    assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
}

pub(super) fn paths(directory: &Path) -> ConfigPaths {
    ConfigPaths::resolve(directory, directory, Some(&directory.join("global")), None)
}

#[test]
fn disabled_capture_performs_no_host_work_and_preserves_opaque_config() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    fs::create_dir_all(&paths.global_dir).unwrap();
    fs::write(
        &paths.global_config,
        r#"{"custom":{"kept":7},"workspace":{"snapshotEnabled":false,"snapshotIntervalMs":0}}"#,
    )
    .unwrap();
    assert!(
        !publish_event(&paths, &sample().server.socket, || panic!(
            "disabled capture"
        ))
        .unwrap()
    );
    assert!(!paths.global_dir.join("workspace").exists());
    let config = ConfigFiles {
        paths: paths.clone(),
    };
    config
        .set(
            tmt_core::settings::Setting::WorkspaceSnapshotEnabled(true),
            tmt_core::settings::Scope::Global,
        )
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(&paths.global_config).unwrap()).unwrap();
    assert_eq!(value["custom"]["kept"], 7);
    assert_eq!(value["workspace"]["snapshotIntervalMs"], 0);
    assert!(
        publish_event(&paths, &sample().server.socket, || Ok(sample())).unwrap(),
        "zero interval does not disable event capture"
    );
}

#[test]
fn concurrent_event_coalesces_without_waiting_and_last_successful_writer_wins() {
    use std::sync::mpsc;
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    let snapshot = sample();
    let socket = snapshot.server.socket.clone();
    assert!(publish_event(&paths, &socket, || Ok(snapshot.clone())).unwrap());
    let latest = paths.workspace_directory(&socket).join("latest.json");
    let (entered, ready) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    std::thread::scope(|scope| {
        let paths = &paths;
        let socket = &socket;
        let snapshot = &snapshot;
        let writer = scope.spawn(move || {
            publish_event(paths, socket, || {
                entered.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
                let mut snapshot = snapshot.clone();
                snapshot.captured_at_ms = 2;
                Ok(snapshot)
            })
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!publish_event(paths, socket, || panic!("contender must skip IO")).unwrap());
        assert_eq!(
            decode(&fs::read(&latest).unwrap()).unwrap().captured_at_ms,
            1
        );
        release.send(()).unwrap();
        assert!(writer.join().unwrap().unwrap());
    });
    assert_eq!(
        decode(&fs::read(&latest).unwrap()).unwrap().captured_at_ms,
        2
    );
    let mut final_snapshot = snapshot;
    final_snapshot.captured_at_ms = 3;
    assert!(publish_event(&paths, &socket, || Ok(final_snapshot.clone())).unwrap());
    assert_eq!(decode(&fs::read(&latest).unwrap()).unwrap(), final_snapshot);
}

#[test]
fn failure_and_unknown_version_preserve_previous_bytes_without_foreign_cleanup() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    let snapshot = sample();
    let socket = &snapshot.server.socket;
    publish_event(&paths, socket, || Ok(snapshot.clone())).unwrap();
    let latest = paths.workspace_directory(socket).join("latest.json");
    let old = fs::read(&latest).unwrap();
    assert!(
        publish_event(&paths, socket, || Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "hung topology"
        )))
        .is_err()
    );
    assert_eq!(fs::read(&latest).unwrap(), old);
    fs::write(&latest, b"{\"version\":2}").unwrap();
    assert!(publish_event(&paths, socket, || panic!("unsupported store")).is_err());
    assert_eq!(fs::read(&latest).unwrap(), b"{\"version\":2}");
}
