use super::*;
use crate::{
    process::{
        CommandError, CommandFailure, CommandOutput, CommandRequest, UnixCommandRunner,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::channel::EnrollmentState,
};
use std::{
    cell::Cell,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};
use tmt_core::endpoint::ProcessIncarnation;

const BINDING: &str = "11111111-1111-4111-8111-111111111111";
const OTHER_BINDING: &str = "44444444-4444-4444-8444-444444444444";
const GENERATION: &str = "22222222-2222-4222-8222-222222222222";
const IDENTITY: &str = "33333333-3333-4333-8333-333333333333";

/// Socket paths are length-limited, so the scratch root stays short.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = PathBuf::from(format!(
            "/tmp/tmt-rc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A real child that recovery must leave running: dropping it kills and reaps
/// it, so no test leaks a process.
struct Running(std::process::Child, ProcessIncarnation);

impl Running {
    fn start() -> Self {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        match observe_runtime_process(&UnixCommandRunner, u64::from(child.id()), deadline()) {
            Ok(ProcessObservation::Live(incarnation)) => Self(child, incarnation),
            other => panic!("the child must be observable: {other:?}"),
        }
    }

    fn still_runs(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A process that has exited and been reaped: conclusively gone.
fn gone() -> Process {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = u64::from(child.id());
    child.wait().unwrap();
    Process {
        pid,
        start: "Thu Oct  1 10:00:00 2026".into(),
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}

fn pane() -> super::super::PaneRecord {
    super::super::PaneRecord {
        host: "tmux".into(),
        server_id: "server-1".into(),
        socket_path: "/tmp/tmux-recovery".into(),
        server_pid: 10,
        server_start_time: "start".into(),
        pane_id: "%7".into(),
        pane_pid: 11,
    }
}

/// An enrollment whose launcher died before it published the foreground.
fn unconfirmed(binding_id: &str) -> Record {
    Record {
        version: RECORD_VERSION,
        binding_id: binding_id.into(),
        identity_id: Some(IDENTITY.into()),
        generation: GENERATION.into(),
        launch_owner: gone(),
        pane: Some(pane()),
        foreground: None,
        claude: None,
    }
}

fn store(directory: &Path, record: &Record) {
    super::super::write_record(directory, record).unwrap();
}

fn bind_socket(directory: &Path, binding_id: &str) -> PathBuf {
    let path = socket_path(directory, binding_id);
    drop(UnixListener::bind(&path).unwrap());
    path
}

fn bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

/// A runner whose every observation fails, so no recorded process can be told.
struct Blind;

impl CommandRunner for Blind {
    fn execute(&self, _: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        Err(CommandError::new(CommandFailure::Timeout))
    }
}

/// Observes for real, but runs `change` once before its first observation: the
/// record changes after recovery read it and before it takes the lock.
struct ChangingRunner<F: Fn()> {
    change: F,
    done: Cell<bool>,
}

impl<F: Fn()> CommandRunner for ChangingRunner<F> {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        if !self.done.replace(true) {
            (self.change)();
        }
        UnixCommandRunner.execute(request)
    }
}

#[test]
fn an_unconfirmed_enrollment_is_inspected_then_recovered_exactly_and_a_repeat_is_a_no_op() {
    let scratch = Scratch::new();
    let directory = scratch.0.as_path();
    store(directory, &unconfirmed(BINDING));
    let socket = bind_socket(directory, BINDING);
    let other = Record {
        foreground: Some(gone()),
        ..unconfirmed(OTHER_BINDING)
    };
    store(directory, &other);
    let other_socket = bind_socket(directory, OTHER_BINDING);
    let other_bytes = bytes(&record_path(directory, OTHER_BINDING));
    fs::write(directory.join(LOCK_FILE), b"").unwrap();

    let report = inspect(&UnixCommandRunner, directory, BINDING, deadline())
        .unwrap()
        .expect("the enrollment is on record");
    assert_eq!(report.state(), EnrollmentState::Unconfirmed);
    assert_eq!(report.generation, GENERATION);
    assert_eq!(report.identity_id.as_deref(), Some(IDENTITY));
    assert_eq!(report.pane.as_ref().unwrap().pane_id, "%7");
    assert_eq!(
        report.removes,
        vec![record_path(directory, BINDING), socket.clone()]
    );
    assert!(report.verification.contains("cannot check it"));
    assert!(report.verification.contains("pane %7"));
    // Inspection is read-only.
    assert!(record_path(directory, BINDING).exists());

    let recovered = recover(
        &UnixCommandRunner,
        directory,
        BINDING,
        GENERATION,
        deadline(),
    );
    let Ok(Recovery::Recovered { removed, kept, .. }) = recovered else {
        panic!("expected recovery: {recovered:?}");
    };
    assert_eq!(
        removed,
        vec![record_path(directory, BINDING), socket.clone()]
    );
    assert!(kept.is_empty());
    assert!(!record_path(directory, BINDING).exists());
    assert!(fs::symlink_metadata(&socket).is_err());
    // The lock and every other enrollment are untouched.
    assert!(directory.join(LOCK_FILE).exists());
    assert_eq!(bytes(&record_path(directory, OTHER_BINDING)), other_bytes);
    assert!(
        fs::symlink_metadata(&other_socket)
            .unwrap()
            .file_type()
            .is_socket()
    );

    assert_eq!(
        recover(
            &UnixCommandRunner,
            directory,
            BINDING,
            GENERATION,
            deadline()
        ),
        Ok(Recovery::Absent)
    );
    assert_eq!(
        inspect(&UnixCommandRunner, directory, BINDING, deadline()),
        Ok(None)
    );
}

#[test]
fn a_present_recorded_process_refuses_recovery_and_is_never_signaled() {
    let scratch = Scratch::new();
    let directory = scratch.0.as_path();
    for role in [
        ProcessRole::LaunchOwner,
        ProcessRole::Foreground,
        ProcessRole::Provider,
    ] {
        let mut agent = Running::start();
        let live = Process::of(&agent.1);
        let record = match role {
            ProcessRole::LaunchOwner => Record {
                launch_owner: live,
                ..unconfirmed(BINDING)
            },
            ProcessRole::Foreground => Record {
                foreground: Some(live),
                ..unconfirmed(BINDING)
            },
            _ => Record {
                claude: Some(live),
                ..unconfirmed(BINDING)
            },
        };
        store(directory, &record);
        let socket = bind_socket(directory, BINDING);
        let before = bytes(&record_path(directory, BINDING));

        let result = recover(
            &UnixCommandRunner,
            directory,
            BINDING,
            GENERATION,
            deadline(),
        );
        let Err(RecoveryError::Running(report)) = result else {
            panic!("{role:?}: expected a refusal: {result:?}");
        };
        assert_eq!(report.state(), EnrollmentState::Running);
        assert!(
            report
                .processes
                .iter()
                .any(|process| process.role == role && process.state == ProcessState::Present)
        );
        assert_eq!(bytes(&record_path(directory, BINDING)), before);
        assert!(
            fs::symlink_metadata(&socket)
                .unwrap()
                .file_type()
                .is_socket()
        );
        assert!(agent.still_runs(), "{role:?}: recovery must never signal");
        fs::remove_file(socket).unwrap();
    }
}

#[test]
fn an_unobservable_process_refuses_recovery_and_leaves_everything() {
    let scratch = Scratch::new();
    let directory = scratch.0.as_path();
    // A failed `ps` alone is not "unobservable": a PID that signal 0 reports as
    // absent is conclusively gone. Only a process that exists but cannot be
    // matched to its incarnation is.
    let mut owner = Running::start();
    store(
        directory,
        &Record {
            launch_owner: Process::of(&owner.1),
            ..unconfirmed(BINDING)
        },
    );
    let socket = bind_socket(directory, BINDING);
    let before = bytes(&record_path(directory, BINDING));

    let result = recover(&Blind, directory, BINDING, GENERATION, deadline());
    let Err(RecoveryError::Unverifiable(report)) = result else {
        panic!("expected a refusal: {result:?}");
    };
    assert_eq!(report.processes[0].state, ProcessState::Unobservable);
    assert_eq!(bytes(&record_path(directory, BINDING)), before);
    assert!(socket.exists());
    assert!(owner.still_runs());
}

#[test]
fn another_generation_or_a_binding_that_is_not_a_uuid_is_left_alone() {
    let scratch = Scratch::new();
    let directory = scratch.0.as_path();
    store(directory, &unconfirmed(BINDING));
    let before = bytes(&record_path(directory, BINDING));

    let other = "55555555-5555-4555-8555-555555555555";
    let result = recover(&UnixCommandRunner, directory, BINDING, other, deadline());
    let Ok(Recovery::OtherGeneration(report)) = result else {
        panic!("expected another generation: {result:?}");
    };
    assert_eq!(report.generation, GENERATION);
    assert_eq!(bytes(&record_path(directory, BINDING)), before);

    // A path-like operand never names a file outside this driver's namespace.
    fs::write(directory.join("x.json"), b"{}").unwrap();
    for binding in ["../x", "x", ""] {
        assert_eq!(
            inspect(&UnixCommandRunner, directory, binding, deadline()),
            Ok(None)
        );
        assert_eq!(
            recover(
                &UnixCommandRunner,
                directory,
                binding,
                GENERATION,
                deadline()
            ),
            Ok(Recovery::Absent)
        );
    }
    assert!(directory.join("x.json").exists());
}

#[test]
fn an_unreadable_record_stays_manual_only_with_a_quoted_command() {
    let scratch = Scratch::new();
    let directory = scratch.0.as_path();
    let path = record_path(directory, BINDING);
    fs::write(&path, b"not json").unwrap();

    let Err(error) = inspect(&UnixCommandRunner, directory, BINDING, deadline()) else {
        panic!("an unreadable record is an error");
    };
    assert_eq!(error.fault, ChannelFault::InvalidRecord);
    assert_eq!(error.path.as_deref(), Some(path.as_path()));
    assert!(error.detail.as_deref().unwrap().contains("rm -- '"));
    let result = recover(
        &UnixCommandRunner,
        directory,
        BINDING,
        GENERATION,
        deadline(),
    );
    assert!(
        matches!(result, Err(RecoveryError::Invalid(_))),
        "{result:?}"
    );
    assert_eq!(bytes(&path), b"not json");
}

#[test]
fn a_record_that_changes_after_it_was_observed_is_left_as_changed() {
    let scratch = Scratch::new();
    let directory = scratch.0.as_path();
    store(directory, &unconfirmed(BINDING));
    let socket = bind_socket(directory, BINDING);
    // The launcher publishes its foreground while recovery observes the record.
    let published = Record {
        foreground: Some(gone()),
        ..unconfirmed(BINDING)
    };
    let runner = ChangingRunner {
        change: || store(directory, &published),
        done: Cell::new(false),
    };

    let result = recover(&runner, directory, BINDING, GENERATION, deadline());
    assert!(
        matches!(result, Err(RecoveryError::Changed(_))),
        "{result:?}"
    );
    assert_eq!(read_record(directory, BINDING), Ok(Some(published)));
    assert!(socket.exists());
}

#[test]
fn an_ended_enrollment_is_recovered_and_a_non_socket_at_the_socket_path_is_kept() {
    let scratch = Scratch::new();
    let directory = scratch.0.as_path();
    let record = Record {
        foreground: Some(gone()),
        claude: Some(gone()),
        ..unconfirmed(BINDING)
    };
    store(directory, &record);
    let squatter = socket_path(directory, BINDING);
    fs::create_dir(&squatter).unwrap();
    fs::write(squatter.join("keep"), b"data").unwrap();

    let report = inspect(&UnixCommandRunner, directory, BINDING, deadline())
        .unwrap()
        .unwrap();
    assert_eq!(report.state(), EnrollmentState::Ended);
    assert_eq!(report.keeps, vec![squatter.clone()]);

    let result = recover(
        &UnixCommandRunner,
        directory,
        BINDING,
        GENERATION,
        deadline(),
    );
    let Ok(Recovery::Recovered { removed, kept, .. }) = result else {
        panic!("expected recovery: {result:?}");
    };
    assert_eq!(removed, vec![record_path(directory, BINDING)]);
    assert_eq!(kept, vec![squatter.clone()]);
    assert_eq!(fs::read(squatter.join("keep")).unwrap(), b"data");
}
