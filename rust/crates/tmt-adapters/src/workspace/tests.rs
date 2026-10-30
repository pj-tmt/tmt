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
    let host = Host::for_caller_with(&caller, runner.clone()).unwrap();
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
            "-u",
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
        &calls[2].args[..8],
        [
            "-u",
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
fn unchanged_capture_preserves_every_published_file_and_timestamp() {
    use std::os::unix::fs::MetadataExt;
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    let mut snapshot = sample();
    let socket = snapshot.server.socket.clone();
    assert!(publish_event(&paths, &socket, || Ok(snapshot.clone())).unwrap());
    let published = || {
        let mut files = fs::read_dir(paths.workspace_directory(&socket))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let metadata = entry.metadata().unwrap();
                (
                    entry.file_name(),
                    fs::read(entry.path()).unwrap(),
                    metadata.ino(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                )
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    };
    let before = published();
    snapshot.captured_at_ms = 60_001;
    assert!(!publish_event(&paths, &socket, || Ok(snapshot)).unwrap());
    assert_eq!(published(), before);
}

#[test]
fn rich_generation_survives_restart_and_repeated_small_captures() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    let mut rich = sample();
    for index in 2..=4 {
        let mut pane = rich.panes[0].clone();
        pane.id = format!("%{index}");
        pane.index = index - 1;
        rich.panes.push(pane);
    }
    let socket = rich.server.socket.clone();
    assert!(publish_event(&paths, &socket, || Ok(rich.clone())).unwrap());
    let rich_bytes = encode(&rich).unwrap();
    for generation in 2..=40 {
        let mut small = sample();
        small.captured_at_ms = generation * 3_600_000;
        small.server.process = ProcessIncarnation::new(11, "restarted-server").unwrap();
        small.windows[0].name = format!("restart-{generation}");
        assert!(publish_event(&paths, &socket, || Ok(small)).unwrap());
    }
    let retained = fs::read_dir(paths.workspace_directory(&socket))
        .unwrap()
        .filter_map(|entry| fs::read(entry.unwrap().path()).ok())
        .any(|bytes| bytes == rich_bytes);
    assert!(
        retained,
        "automatic captures discarded the pre-crash layout"
    );
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
                snapshot.windows[0].name = "second layout".into();
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
    final_snapshot.windows[0].name = "third layout".into();
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

#[test]
fn command_interval_skips_capture_until_due_and_failed_refresh_preserves_bytes() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    let snapshot = sample();
    let socket = &snapshot.server.socket;
    assert!(publish_event(&paths, socket, || Ok(snapshot.clone())).unwrap());
    let latest = paths.workspace_directory(socket).join("latest.json");
    let old = fs::read(&latest).unwrap();
    assert!(!publish_refresh(&paths, socket, 60_001, || panic!("fresh timestamp")).unwrap());
    assert!(
        publish_refresh(&paths, socket, 60_002, || Err(io::Error::other(
            "capture refused"
        )))
        .is_err()
    );
    assert_eq!(fs::read(&latest).unwrap(), old);
    assert!(
        publish_refresh(&paths, socket, 60_002, || {
            let mut updated = snapshot.clone();
            updated.captured_at_ms = 60_002;
            updated.windows[0].name = "changed layout".into();
            Ok(updated)
        })
        .unwrap()
    );
    assert!(!publish_refresh(&paths, socket, 60_003, || panic!("updated timestamp")).unwrap());
    ConfigFiles {
        paths: paths.clone(),
    }
    .set(
        tmt_core::settings::Setting::WorkspaceSnapshotIntervalMs(2),
        tmt_core::settings::Scope::Global,
    )
    .unwrap();
    assert!(publish_refresh(&paths, socket, 60_005, || Ok(snapshot.clone())).unwrap());
    ConfigFiles {
        paths: paths.clone(),
    }
    .set(
        tmt_core::settings::Setting::WorkspaceSnapshotIntervalMs(0),
        tmt_core::settings::Scope::Global,
    )
    .unwrap();
    assert!(!publish_refresh(&paths, socket, u64::MAX, || panic!("event only")).unwrap());
    assert!(
        publish_event(&paths, socket, || {
            let mut event = snapshot.clone();
            event.windows[0].name = "event capture".into();
            Ok(event)
        })
        .unwrap()
    );
}

#[test]
fn command_contender_skips_and_observes_the_committed_timestamp_next_time() {
    use std::sync::mpsc;
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    let socket = sample().server.socket;
    let (entered, ready) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    std::thread::scope(|scope| {
        let paths = &paths;
        let socket = &socket;
        let writer = scope.spawn(move || {
            publish_refresh(paths, socket, 100_000, || {
                entered.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
                let mut snapshot = sample();
                snapshot.captured_at_ms = 100_000;
                Ok(snapshot)
            })
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!publish_refresh(paths, socket, 100_000, || panic!("contender capture")).unwrap());
        release.send(()).unwrap();
        assert!(writer.join().unwrap().unwrap());
    });
    assert!(
        !publish_refresh(&paths, &socket, 100_000, || panic!(
            "committed fresh timestamp"
        ))
        .unwrap()
    );
}

#[test]
fn command_without_selected_tmux_pane_creates_no_snapshot_or_database() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    let caller = CallerEnvironment {
        tmux: Some("/tmp/example/socket,10,0".into()),
        pane: None,
        process_id: 20,
        driver_env: Default::default(),
    };
    let host = Host::for_caller_with(&caller, crate::process::UnixCommandRunner).unwrap();
    assert!(!refresh_command(&paths, &host, &caller, Instant::now() + CAPTURE_BUDGET).unwrap());
    assert!(!paths.global_dir.exists());
    assert!(!paths.database.exists());
}

/// Every byte a tmux store publishes, across changed, shrinking and unchanged captures:
/// the documents, the index and the legacy projection, by filename (#5666 refactor guard).
fn tmux_store_fingerprint() -> String {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    let mut snapshot = sample();
    snapshot.panes[0].identity = Some(tmt_core::workspace::WorkspaceIdentity {
        id: "4f1e7a6c-5a2b-4c3d-8e9f-0a1b2c3d4e5f".into(),
        name: "seat".into(),
        lifetime: "saved".into(),
        binding: "binding-1".into(),
        harness: Some("codex".into()),
        session: Some("session-1".into()),
        mode: Some("default".into()),
        channel: Some(true),
    });
    let socket = snapshot.server.socket.clone();
    // Losing the one annotated identity is a shrink; the window rename is a layout change.
    let mut wide = snapshot.clone();
    wide.panes[0].identity = None;
    wide.windows[0].name = "renamed".into();
    for (at, document) in [(1, &snapshot), (2, &wide), (3, &snapshot), (4, &snapshot)] {
        let mut document = document.clone();
        document.captured_at_ms = at;
        publish_event(&paths, &socket, || Ok(document)).unwrap();
    }
    let store = paths.workspace_directory(&socket);
    let mut files = std::fs::read_dir(&store)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.file_name().unwrap() != "publication.lock")
        .map(|path| {
            (
                path.file_name().unwrap().to_owned(),
                std::fs::read(&path).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    files.sort();
    let mut all = Vec::new();
    for (name, bytes) in files {
        all.extend(name.as_encoded_bytes());
        all.push(0);
        all.extend(bytes);
        all.push(0);
    }
    tmt_core::content_digest::sha256(&all)
}

#[test]
fn tmux_store_bytes_are_unchanged_by_the_shared_store_refactor() {
    // Computed on main before #5666 made the store generic over its document kind.
    assert_eq!(
        tmux_store_fingerprint(),
        "75d88e89229ac65c648eb2e7fae752adb5b6bdc6bd05403705d4f14478a1f0de"
    );
}

fn member(id: &str) -> builtin::BuiltinMember {
    builtin::BuiltinMember {
        cwd: Some(format!("/work/{id}")),
        identity: WorkspaceIdentity {
            id: id.into(),
            name: format!("name-{id}"),
            lifetime: "saved".into(),
            binding: format!("binding-{id}"),
            harness: Some("codex".into()),
            session: Some(format!("session-{id}")),
            mode: Some("independent".into()),
            channel: None,
        },
    }
}

fn members(at: u64, ids: &[&str]) -> builtin::BuiltinSnapshot {
    builtin::BuiltinSnapshot {
        captured_at_ms: at,
        members: ids.iter().map(|id| member(id)).collect(),
    }
}

fn publish_builtin(paths: &ConfigPaths, snapshot: builtin::BuiltinSnapshot) -> bool {
    publish_store_event(paths, Store::Builtin, || Ok(snapshot)).unwrap()
}

fn protected_members(paths: &ConfigPaths) -> Vec<String> {
    let document =
        saved::acquire_selected::<builtin::BuiltinSnapshot>(paths, Store::Builtin, None, None)
            .unwrap();
    assert!(
        document.protected,
        "the default selection is the protected baseline"
    );
    document
        .snapshot
        .members
        .into_iter()
        .map(|member| member.identity.id)
        .collect()
}

#[test]
fn built_in_codec_round_trips_and_neither_codec_admits_the_other_kind() {
    let snapshot = members(5, &["a", "b"]);
    let bytes = encode_builtin(&snapshot).unwrap();
    assert_eq!(decode_builtin(&bytes).unwrap(), snapshot);
    assert!(decode(&bytes).is_err());
    assert!(decode_builtin(&encode(&sample()).unwrap()).is_err());
    let mut repeated = snapshot.clone();
    repeated.members[1].identity.binding = "binding-a".into();
    assert!(encode_builtin(&repeated).is_err());
    let text = String::from_utf8(bytes).unwrap();
    for broken in [
        text.replace("\"pty\"", "\"tmux\""),
        text.replace("\"version\":1", "\"version\":2"),
        text.replace("\"members\":[", "\"members\":{")
            .replace("]}", "}}"),
    ] {
        assert!(decode_builtin(broken.as_bytes()).is_err(), "{broken}");
    }
}

#[test]
fn logout_keeps_the_full_built_in_baseline_and_an_unchanged_capture_writes_nothing() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    assert!(publish_builtin(&paths, members(1, &["a", "b", "c", "d"])));
    let store = paths.builtin_workspace_directory();
    let before = fs::read(store.join("generations.json")).unwrap();
    assert!(!publish_builtin(&paths, members(2, &["a", "b", "c", "d"])));
    assert_eq!(fs::read(store.join("generations.json")).unwrap(), before);
    // Members end one by one: each partial capture is retained but none becomes the baseline.
    for (at, ids) in [
        (3, &["a", "b", "c"][..]),
        (4, &["a", "b"]),
        (5, &["a"]),
        (6, &[]),
    ] {
        assert!(publish_builtin(&paths, members(at, ids)));
    }
    assert_eq!(protected_members(&paths), ["a", "b", "c", "d"]);
    let listed = saved::list_snapshots::<builtin::BuiltinSnapshot>(&paths, Store::Builtin).unwrap();
    assert_eq!(listed[0].panes, 0);
    assert!(listed[0].generation.as_ref().unwrap().shrink);
}

#[test]
fn an_explicit_stop_leaves_the_baseline_and_a_new_member_then_advances_it() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    assert!(
        !forget_builtin_member(&paths, "a").unwrap(),
        "no store is created"
    );
    assert!(!paths.builtin_workspace_directory().exists());
    assert!(publish_builtin(&paths, members(1, &["a", "b", "c", "d"])));
    // d is stopped on purpose; its supervisor's end is then captured.
    assert!(forget_builtin_member(&paths, "d").unwrap());
    assert!(!forget_builtin_member(&paths, "d").unwrap());
    assert_eq!(protected_members(&paths), ["a", "b", "c"]);
    assert!(
        !publish_builtin(&paths, members(2, &["a", "b", "c"])),
        "nothing changed"
    );
    // c is stopped and e started: the capture holds the remaining baseline and advances it.
    assert!(forget_builtin_member(&paths, "c").unwrap());
    assert!(publish_builtin(&paths, members(3, &["a", "b", "e"])));
    assert_eq!(protected_members(&paths), ["a", "b", "e"]);
    // A capture that also lost an unexplained member is a shrink.
    assert!(publish_builtin(&paths, members(4, &["a", "e"])));
    assert_eq!(protected_members(&paths), ["a", "b", "e"]);
}

#[test]
fn a_member_started_after_an_unexplained_end_is_protected_at_once() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    // 1. The baseline is {a, b, c}.
    assert!(publish_builtin(&paths, members(1, &["a", "b", "c"])));
    // 2. c exits on its own (no tmt stop): a shrink that keeps c.
    assert!(publish_builtin(&paths, members(2, &["a", "b"])));
    assert_eq!(protected_members(&paths), ["a", "b", "c"]);
    // 3. d starts; 4. the capture {a, b, d} merges into the baseline at once.
    assert!(publish_builtin(&paths, members(3, &["a", "b", "d"])));
    assert_eq!(protected_members(&paths), ["a", "b", "c", "d"]);
    // The same live set again settles: one more shrink record, then nothing to write.
    assert!(publish_builtin(&paths, members(4, &["a", "b", "d"])));
    assert!(!publish_builtin(&paths, members(5, &["a", "b", "d"])));
    // 5. After a reboot the default restore has d; c stays until it is stopped or removed.
    assert_eq!(protected_members(&paths), ["a", "b", "c", "d"]);
    assert!(forget_builtin_member(&paths, "c").unwrap());
    assert_eq!(protected_members(&paths), ["a", "b", "d"]);
    // Logout afterwards still keeps everyone.
    for (at, ids) in [(6, &["a", "b"][..]), (7, &["a"]), (8, &[])] {
        assert!(publish_builtin(&paths, members(at, ids)));
    }
    assert_eq!(protected_members(&paths), ["a", "b", "d"]);
}

#[test]
fn built_in_named_saves_list_and_remove_beside_the_generations() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    assert!(
        saved::list_snapshots::<builtin::BuiltinSnapshot>(&paths, Store::Builtin)
            .unwrap()
            .is_empty()
    );
    assert!(publish_builtin(&paths, members(1, &["a"])));
    let name = "before-reboot".parse().unwrap();
    let entry = saved::save::<builtin::BuiltinSnapshot>(&paths, Store::Builtin, &name).unwrap();
    assert_eq!((entry.panes, entry.agents), (1, 1));
    assert!(matches!(
        saved::save::<builtin::BuiltinSnapshot>(&paths, Store::Builtin, &name),
        Err(saved::Error::Exists)
    ));
    let listed = saved::list_snapshots::<builtin::BuiltinSnapshot>(&paths, Store::Builtin).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1].name.as_ref(), Some(&name));
    // A tmux reader never admits the built-in store's bytes.
    assert!(
        saved::acquire::<WorkspaceSnapshot>(&paths, Store::Builtin, Some(&name), None).is_err()
    );
    saved::remove::<builtin::BuiltinSnapshot>(&paths, Store::Builtin, &name).unwrap();
    assert_eq!(
        saved::list_snapshots::<builtin::BuiltinSnapshot>(&paths, Store::Builtin)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn socket_stores_name_each_tmux_store_by_its_recorded_socket_and_never_the_built_in_one() {
    let directory = crate::test_support::TestDirectory::new();
    let paths = paths(&directory.path);
    assert!(saved::socket_stores(&paths).is_empty());
    assert!(publish_builtin(&paths, members(1, &["a"])));
    assert!(saved::socket_stores(&paths).is_empty());
    let mut first = sample();
    first.server.socket = "/tmp/b.sock".into();
    let mut second = sample();
    second.server.socket = "/tmp/a.sock".into();
    for snapshot in [first, second] {
        let socket = snapshot.server.socket.clone();
        assert!(publish_event(&paths, &socket, || Ok(snapshot)).unwrap());
    }
    // A foreign directory with a digest name but no readable document is left out.
    fs::create_dir(
        paths
            .global_dir
            .join("workspace")
            .join(tmt_core::content_digest::sha256(b"/tmp/foreign.sock")),
    )
    .unwrap();
    assert_eq!(saved::socket_stores(&paths), ["/tmp/a.sock", "/tmp/b.sock"]);
}
