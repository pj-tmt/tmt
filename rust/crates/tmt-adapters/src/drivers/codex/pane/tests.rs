use super::super::record::{Attribution, Process, Ready, Record};
use super::*;
use crate::test_support::TestDirectory;
use std::{cell::Cell, time::Duration};
use tmt_core::{endpoint::ServerEvidence, host::HostKind};
fn server() -> ServerEvidence {
    ServerEvidence {
        host: HostKind::Tmux,
        server_id: "44444444-4444-4444-8444-444444444444".into(),
        socket_path: "/owned/tmux.sock".into(),
        server_pid: 7,
        server_start_time: "server-start".into(),
    }
}
fn record(server: &ServerEvidence) -> Record {
    let mut record = Record::new(
        "11111111-1111-4111-8111-111111111111",
        &ProcessIncarnation::new(42, "owner").unwrap(),
    )
    .unwrap();
    record.attribution =
        Some(Attribution::new("33333333-3333-4333-8333-333333333333", server, "%2", 8).unwrap());
    record
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
#[test]
fn unknown_after_endpoint_cleanup_is_terminal_only_for_exact_attributed_pane() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let server = server();
    let mut record = record(&server);
    record.ready = Some(Ready {
        server: Process {
            pid: 43,
            start: "app-server".into(),
        },
        port: 49000,
        thread: record.binding_id.clone(),
    });
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    let pane = PaneAddress {
        server: &server,
        pane_id: "%2",
        pane_pid: 8,
    };
    let before = fs::read(
        fixture
            .path
            .join("codex")
            .join(format!("{}.json", record.binding_id)),
    )
    .unwrap();
    let error = inspect(&fixture.path, &pane, None, deadline(), |_| {
        RuntimeLiveness::Gone
    })
    .unwrap_err();
    assert_eq!(error.fault, ChannelFault::Unverifiable);
    assert!(error.message().contains("original foreground"));
    assert!(error.message().contains("%2"));
    assert!(error.message().contains(&format!(
        "tmt channel recover --binding {} --generation {}",
        record.binding_id, record.generation
    )));
    assert!(
        !inspect(
            &fixture.path,
            &PaneAddress {
                pane_pid: 9,
                ..pane
            },
            None,
            deadline(),
            |_| panic!("another pane must not probe these processes")
        )
        .unwrap()
        .enrolled
    );
    let mut replacement_server = server.clone();
    replacement_server.server_start_time.push('x');
    assert!(
        !inspect(
            &fixture.path,
            &PaneAddress {
                server: &replacement_server,
                ..pane
            },
            None,
            deadline(),
            |_| panic!("different server incarnation")
        )
        .unwrap()
        .enrolled
    );
    assert_eq!(
        fs::read(error.path.unwrap()).unwrap(),
        before,
        "pane evidence is read-only"
    );
}
#[test]
fn exact_foreground_survives_owner_loss_and_only_positive_end_permits_baseline() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let server = server();
    let record = record(&server);
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    store
        .foreground(&record, &ProcessIncarnation::new(44, "foreground").unwrap())
        .unwrap();
    let pane = PaneAddress {
        server: &server,
        pane_id: "%2",
        pane_pid: 8,
    };
    // No ancestry is needed: reparenting cannot make the exact address disappear.
    assert!(
        inspect(&fixture.path, &pane, None, deadline(), |p| {
            if p.pid() == 44 {
                RuntimeLiveness::Alive
            } else {
                RuntimeLiveness::Gone
            }
        })
        .unwrap()
        .enrolled
    );
    assert!(
        inspect(&fixture.path, &pane, None, deadline(), |p| {
            if p.pid() == 44 {
                RuntimeLiveness::Unknown
            } else {
                RuntimeLiveness::Gone
            }
        })
        .is_err()
    );
    assert!(
        !inspect(&fixture.path, &pane, None, deadline(), |_| {
            RuntimeLiveness::Gone
        })
        .unwrap()
        .enrolled
    );
}
#[test]
fn unattributed_and_corrupt_records_warn_but_current_binding_corruption_is_terminal() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let server = server();
    let mut record = record(&server);
    record.attribution = None;
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    let pane = PaneAddress {
        server: &server,
        pane_id: "%2",
        pane_pid: 8,
    };
    let path = fixture
        .path
        .join("codex")
        .join(format!("{}.json", record.binding_id));
    for corrupt in [false, true] {
        if corrupt {
            crate::private_file::replace(&path, b"{").unwrap();
        }
        let result = inspect(&fixture.path, &pane, None, deadline(), |_| {
            panic!("unattributed evidence must not probe")
        })
        .unwrap();
        assert!(!result.enrolled);
        assert_eq!(result.skipped, vec![path.clone()]);
        let error = inspect(
            &fixture.path,
            &pane,
            Some(&record.binding_id),
            deadline(),
            |_| RuntimeLiveness::Gone,
        )
        .unwrap_err();
        assert_eq!(error.path.as_ref(), Some(&path));
        assert!(error.message().contains(&record.binding_id));
    }
}
#[test]
fn more_than_256_ended_records_do_not_exhaust_an_active_record_cap() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let server = server();
    for _ in 0..260 {
        let mut record = record(&server);
        record.binding_id = uuid::Uuid::new_v4().to_string();
        record.foreground = Foreground::Known(Process {
            pid: 44,
            start: "ended".into(),
        });
        store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    }
    let calls = Cell::new(0);
    let result = inspect(
        &fixture.path,
        &PaneAddress {
            server: &server,
            pane_id: "%2",
            pane_pid: 8,
        },
        None,
        deadline(),
        |_| {
            calls.set(calls.get() + 1);
            RuntimeLiveness::Gone
        },
    )
    .unwrap();
    assert!(!result.enrolled);
    assert!(result.skipped.is_empty());
    assert_eq!(calls.get(), 520);
}
#[test]
fn enrollment_refuses_missing_attribution_before_starting_anything() {
    use crate::runtime::{
        RuntimeCommand,
        channel::{ChannelError, ChannelPlan, RuntimeChannel},
    };
    let fixture = TestDirectory::new();
    let server = server();
    let owner = ProcessIncarnation::new(42, "owner").unwrap();
    let command = RuntimeCommand {
        executable: "/must-not-spawn".into(),
        args: vec![],
    };
    let plan = ChannelPlan {
        binding_id: "11111111-1111-4111-8111-111111111111",
        identity_id: "33333333-3333-4333-8333-333333333333",
        pane: PaneAddress {
            server: &server,
            pane_id: "",
            pane_pid: 0,
        },
        owner: &owner,
        command: &command,
        working_directory: &fixture.path,
        tmt: Path::new("/must-not-spawn"),
        directory: &fixture.path,
    };
    assert!(matches!(
        super::super::channel::CodexChannel.enroll(&plan),
        Err(ChannelError::Unattributed)
    ));
    assert!(!fixture.path.join("codex").exists());
}

#[test]
fn unsupported_permissions_refuse_before_spawn_or_record_creation() {
    use crate::runtime::{
        RuntimeCommand,
        channel::{ChannelError, ChannelPlan, RuntimeChannel},
    };
    let fixture = TestDirectory::new();
    let server = server();
    let owner = ProcessIncarnation::new(42, "owner").unwrap();
    for args in [
        vec!["-s", "unknown"],
        vec!["-a", "on-failure"],
        vec!["-c", "permissions.profile=true"],
        vec!["-c", "approval_policy=\"never\""],
    ] {
        let command = RuntimeCommand {
            // Any attempted spawn would fail with a different error. The typed
            // argument refusal and absent store establish the earlier boundary.
            executable: fixture.path.join("must-not-be-spawned").into_os_string(),
            args: args.iter().map(std::ffi::OsString::from).collect(),
        };
        let plan = ChannelPlan {
            binding_id: "11111111-1111-4111-8111-111111111111",
            identity_id: "33333333-3333-4333-8333-333333333333",
            pane: PaneAddress {
                server: &server,
                pane_id: "%2",
                pane_pid: 8,
            },
            owner: &owner,
            command: &command,
            working_directory: &fixture.path,
            tmt: Path::new("/must-not-spawn-supervisor"),
            directory: &fixture.path,
        };
        assert!(
            matches!(
                super::super::channel::CodexChannel.enroll(&plan),
                Err(ChannelError::UnsupportedArguments(_))
            ),
            "{args:?}"
        );
        assert!(
            !fixture.path.join("codex").exists(),
            "refusal must not create enrollment state"
        );
    }
}

#[test]
fn measured_native_lookup_caches_repeated_recorded_pid() {
    use crate::process::{CommandError, CommandOutput, CommandRequest, CommandRunner};
    struct Measured(Cell<usize>);
    impl CommandRunner for Measured {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            self.0.set(self.0.get() + 1);
            UnixCommandRunner.execute(request)
        }
    }
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let server = server();
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    let mut record = record(&server);
    record.launch_owner = Process {
        pid: pid.into(),
        start: "exited-owned-test-child".into(),
    };
    record.foreground = Foreground::Known(record.launch_owner.clone());
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    let measured = Measured(Cell::new(0));
    let began = Instant::now();
    let answer = enrolled_with_runner(
        &measured,
        &fixture.path,
        &PaneAddress {
            server: &server,
            pane_id: "%2",
            pane_pid: 8,
        },
        None,
        deadline(),
    )
    .unwrap();
    assert!(!answer.enrolled);
    assert_eq!(measured.0.get(), 1);
    eprintln!(
        "Codex pane evidence: {} us, {} native ps command, two equal recorded PIDs, no global process snapshot",
        began.elapsed().as_micros(),
        measured.0.get()
    );
}
