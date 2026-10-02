use super::super::record::{Attribution, Ready};
use super::*;
use crate::{
    process::{
        UnixCommandRunner,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    test_support::TestDirectory,
};
use std::{
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, symlink},
    time::Duration,
};
use tmt_core::{
    binding::session::RuntimeLiveness,
    endpoint::{ProcessIncarnation, ServerEvidence},
    host::HostKind,
};

const BINDING: &str = "11111111-1111-4111-8111-111111111111";
const OTHER_BINDING: &str = "55555555-5555-4555-8555-555555555555";

/// A real child that recovery must leave running; dropping it kills and reaps it.
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

/// An exited, reaped process: conclusively gone.
fn gone() -> ProcessIncarnation {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = u64::from(child.id());
    child.wait().unwrap();
    ProcessIncarnation::new(pid, "Thu Oct  1 10:00:00 2026").unwrap()
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}

fn server() -> ServerEvidence {
    ServerEvidence {
        host: HostKind::Tmux,
        server_id: "44444444-4444-4444-8444-444444444444".into(),
        socket_path: "/owned/tmux.sock".into(),
        server_pid: 7,
        server_start_time: "server-start".into(),
    }
}

/// An enrollment whose launcher died before it published the foreground; the
/// app-server became ready (`endpoint`) or never did (`None`).
fn enrollment(
    store: &Store,
    binding_id: &str,
    owner: &ProcessIncarnation,
    endpoint: Option<&ProcessIncarnation>,
) -> Record {
    let mut record = Record::new(binding_id, owner).unwrap();
    record.attribution =
        Some(Attribution::new("33333333-3333-4333-8333-333333333333", &server(), "%2", 8).unwrap());
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    match endpoint {
        Some(endpoint) => store
            .ready(
                &record,
                Ready {
                    server: Process::of(endpoint),
                    port: 49000,
                    thread: "22222222-2222-4222-8222-222222222222".into(),
                },
            )
            .unwrap(),
        None => record,
    }
}

/// The files a launch leaves in its generation directory when its supervisor
/// could not clean up.
fn generation_files(store: &Store, record: &Record) -> PathBuf {
    let directory = store.generation_directory(record).unwrap();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    for name in [CAPABILITY_FILE, LOG_FILE] {
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(directory.join(name))
            .unwrap();
    }
    fs::write(directory.join(CAPABILITY_FILE), b"secret").unwrap();
    directory
}

fn bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

#[test]
fn an_unconfirmed_enrollment_with_a_gone_app_server_is_recovered_with_its_generation_files() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let record = enrollment(&store, BINDING, &gone(), Some(&gone()));
    let generation = generation_files(&store, &record);
    let other = enrollment(&store, OTHER_BINDING, &gone(), Some(&gone()));
    let other_generation = generation_files(&store, &other);
    let other_bytes = bytes(&store.path(OTHER_BINDING).unwrap());

    let report = inspect(&UnixCommandRunner, &fixture.path, BINDING, deadline())
        .unwrap()
        .expect("the enrollment is on record");
    assert_eq!(report.state(), EnrollmentState::Unconfirmed);
    assert_eq!(report.generation, record.generation);
    assert_eq!(report.pane.as_ref().unwrap().pane_id, "%2");
    assert!(
        report
            .processes
            .iter()
            .any(|process| process.role == ProcessRole::Endpoint)
    );
    assert!(report.verification.contains("cannot check it"));
    assert_eq!(
        report.removes,
        vec![
            store.path(BINDING).unwrap(),
            generation.join(CAPABILITY_FILE),
            generation.join(LOG_FILE),
            generation.clone(),
        ]
    );
    assert!(
        store.path(BINDING).unwrap().exists(),
        "inspect is read-only"
    );

    let result = recover(
        &UnixCommandRunner,
        &fixture.path,
        BINDING,
        &record.generation,
        deadline(),
    );
    let Ok(Recovery::Recovered { removed, kept, .. }) = result else {
        panic!("expected recovery: {result:?}");
    };
    assert_eq!(removed, report.removes);
    assert!(kept.is_empty());
    assert!(!store.path(BINDING).unwrap().exists());
    assert!(fs::symlink_metadata(&generation).is_err());
    assert!(
        store.path(BINDING).unwrap().with_extension("lock").exists(),
        "the lock inode is retained"
    );
    assert_eq!(bytes(&store.path(OTHER_BINDING).unwrap()), other_bytes);
    assert_eq!(bytes(&other_generation.join(CAPABILITY_FILE)), b"secret");

    assert_eq!(
        recover(
            &UnixCommandRunner,
            &fixture.path,
            BINDING,
            &record.generation,
            deadline()
        ),
        Ok(Recovery::Absent)
    );
}

#[test]
fn an_app_server_that_was_never_recorded_keeps_the_generation_directory() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let record = enrollment(&store, BINDING, &gone(), None);
    let generation = generation_files(&store, &record);

    let report = inspect(&UnixCommandRunner, &fixture.path, BINDING, deadline())
        .unwrap()
        .unwrap();
    assert!(
        report
            .verification
            .contains("app-server was never recorded")
    );
    assert_eq!(report.removes, vec![store.path(BINDING).unwrap()]);
    assert_eq!(report.keeps, vec![generation.clone()]);

    let result = recover(
        &UnixCommandRunner,
        &fixture.path,
        BINDING,
        &record.generation,
        deadline(),
    );
    let Ok(Recovery::Recovered { removed, kept, .. }) = result else {
        panic!("expected recovery: {result:?}");
    };
    assert_eq!(removed, vec![store.path(BINDING).unwrap()]);
    assert_eq!(kept, vec![generation.clone()]);
    assert_eq!(bytes(&generation.join(CAPABILITY_FILE)), b"secret");
    assert!(generation.join(LOG_FILE).exists());
}

#[test]
fn unknown_entries_and_links_in_a_proven_generation_directory_are_kept_never_recursed() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let record = enrollment(&store, BINDING, &gone(), Some(&gone()));
    let generation = generation_files(&store, &record);
    // The log is replaced by a link to a file outside, and a stranger is added.
    let outside = fixture.path.join("outside");
    fs::write(&outside, b"keep").unwrap();
    fs::remove_file(generation.join(LOG_FILE)).unwrap();
    symlink(&outside, generation.join(LOG_FILE)).unwrap();
    fs::create_dir(generation.join("nested")).unwrap();
    fs::write(generation.join("nested").join("file"), b"keep").unwrap();

    let result = recover(
        &UnixCommandRunner,
        &fixture.path,
        BINDING,
        &record.generation,
        deadline(),
    );
    let Ok(Recovery::Recovered { removed, kept, .. }) = result else {
        panic!("expected recovery: {result:?}");
    };
    assert_eq!(
        removed,
        vec![
            store.path(BINDING).unwrap(),
            generation.join(CAPABILITY_FILE)
        ]
    );
    assert!(kept.contains(&generation.join(LOG_FILE)));
    assert!(kept.contains(&generation.join("nested")));
    assert!(kept.contains(&generation));
    assert_eq!(bytes(&outside), b"keep");
    assert_eq!(bytes(&generation.join("nested").join("file")), b"keep");
}

#[test]
fn a_present_foreground_owner_or_app_server_refuses_and_is_never_signaled() {
    for role in [
        ProcessRole::LaunchOwner,
        ProcessRole::Foreground,
        ProcessRole::Endpoint,
    ] {
        let fixture = TestDirectory::new();
        let store = Store::open(&fixture.path).unwrap();
        let mut live = Running::start();
        let record = match role {
            ProcessRole::LaunchOwner => enrollment(&store, BINDING, &live.1, Some(&gone())),
            ProcessRole::Endpoint => enrollment(&store, BINDING, &gone(), Some(&live.1)),
            _ => {
                let record = enrollment(&store, BINDING, &gone(), Some(&gone()));
                store.foreground(&record, &live.1).unwrap()
            }
        };
        let generation = generation_files(&store, &record);
        let before = bytes(&store.path(BINDING).unwrap());

        let result = recover(
            &UnixCommandRunner,
            &fixture.path,
            BINDING,
            &record.generation,
            deadline(),
        );
        let Err(RecoveryError::Running(report)) = result else {
            panic!("{role:?}: expected a refusal: {result:?}");
        };
        assert!(
            report
                .processes
                .iter()
                .any(|process| process.role == role && process.state == ProcessState::Present)
        );
        assert_eq!(bytes(&store.path(BINDING).unwrap()), before);
        assert_eq!(bytes(&generation.join(CAPABILITY_FILE)), b"secret");
        assert!(live.still_runs(), "{role:?}: recovery must never signal");
    }
}

#[test]
fn an_ended_enrollment_is_recovered_and_another_generation_is_left_alone() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let record = enrollment(&store, BINDING, &gone(), Some(&gone()));
    let record = store.foreground(&record, &gone()).unwrap();
    let before = bytes(&store.path(BINDING).unwrap());

    let other = uuid::Uuid::new_v4().to_string();
    let result = recover(
        &UnixCommandRunner,
        &fixture.path,
        BINDING,
        &other,
        deadline(),
    );
    assert!(
        matches!(&result, Ok(Recovery::OtherGeneration(report)) if report.generation == record.generation),
        "{result:?}"
    );
    assert_eq!(bytes(&store.path(BINDING).unwrap()), before);

    let report = inspect(&UnixCommandRunner, &fixture.path, BINDING, deadline())
        .unwrap()
        .unwrap();
    assert_eq!(report.state(), EnrollmentState::Ended);
    assert!(matches!(
        recover(
            &UnixCommandRunner,
            &fixture.path,
            BINDING,
            &record.generation,
            deadline()
        ),
        Ok(Recovery::Recovered { .. })
    ));
    assert!(!store.path(BINDING).unwrap().exists());
}

#[test]
fn an_unreadable_record_stays_manual_only_and_a_non_uuid_names_nothing() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let path = store.path(BINDING).unwrap();
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    fs::write(&path, b"not json").unwrap();

    let Err(error) = inspect(&UnixCommandRunner, &fixture.path, BINDING, deadline()) else {
        panic!("an unreadable record is an error");
    };
    assert_eq!(error.fault, ChannelFault::InvalidRecord);
    assert!(error.detail.as_deref().unwrap().contains("rm -- '"));
    let generation = uuid::Uuid::new_v4().to_string();
    let result = recover(
        &UnixCommandRunner,
        &fixture.path,
        BINDING,
        &generation,
        deadline(),
    );
    assert!(
        matches!(result, Err(RecoveryError::Invalid(_))),
        "{result:?}"
    );
    assert_eq!(bytes(&path), b"not json");

    for binding in ["../x", "x"] {
        assert_eq!(
            inspect(&UnixCommandRunner, &fixture.path, binding, deadline()),
            Ok(None)
        );
    }
}

#[test]
fn a_known_file_that_cannot_be_removed_stops_cleanup_and_names_it() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let record = enrollment(&store, BINDING, &gone(), Some(&gone()));
    let generation = generation_files(&store, &record);
    let capability = generation.join(CAPABILITY_FILE);

    let result = clean_with(&generation, |path| {
        if path == capability {
            Err(io::ErrorKind::PermissionDenied.into())
        } else {
            fs::remove_file(path)
        }
    });
    assert_eq!(result, Err(capability.clone()));
    assert_eq!(bytes(&capability), b"secret");
    assert!(
        generation.is_dir(),
        "the directory stays while a file remains"
    );
}

#[test]
fn a_failed_cleanup_keeps_the_record_so_the_same_recovery_can_finish_later() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let record = enrollment(&store, BINDING, &gone(), Some(&gone()));
    let path = store.path(BINDING).unwrap();
    let before = bytes(&path);
    let stuck = fixture.path.join("stuck");

    let retained = store
        .recover(&record, || Err::<(), _>(stuck.clone()))
        .unwrap();
    assert!(matches!(&retained, Removal::Retained(named) if *named == stuck));
    assert_eq!(bytes(&path), before, "the record stays for a retry");

    let removed = store.recover(&record, || Ok::<_, PathBuf>(())).unwrap();
    assert!(matches!(removed, Removal::Removed(())));
    assert!(!path.exists());
}
