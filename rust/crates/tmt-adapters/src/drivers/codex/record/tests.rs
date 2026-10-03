use super::*;
use crate::test_support::TestDirectory;
use std::os::unix::fs::{PermissionsExt, symlink};

fn record() -> Record {
    Record::new(
        "11111111-1111-4111-8111-111111111111",
        &ProcessIncarnation::new(42, "launch-start").unwrap(),
    )
    .unwrap()
}

#[test]
fn opt_in_precedes_readiness_and_exact_lease_withdraws_once() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let enrollment = record();
    assert!(store.read(&enrollment.binding_id).unwrap().is_none());
    store
        .create(&enrollment, |_| RuntimeLiveness::Alive)
        .unwrap();
    assert!(
        store
            .read(&enrollment.binding_id)
            .unwrap()
            .unwrap()
            .ready
            .is_none()
    );
    let ready = store
        .ready(
            &enrollment,
            Ready {
                server: Process::of(&ProcessIncarnation::new(43, "server-start").unwrap()),
                port: 49000,
                thread: "22222222-2222-4222-8222-222222222222".into(),
            },
        )
        .unwrap();
    assert!(store.read(&enrollment.binding_id).unwrap() == Some(ready));
    assert!(store.withdraw(&enrollment).unwrap());
    assert!(!store.withdraw(&enrollment).unwrap());
    assert!(
        store
            .directory
            .join(format!("{}.lock", enrollment.binding_id))
            .exists(),
        "stable lock inode is retained"
    );
}

#[test]
fn another_generation_or_owner_is_never_removed_or_marked_ready() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let enrollment = record();
    store
        .create(&enrollment, |_| RuntimeLiveness::Alive)
        .unwrap();
    let ready = Ready {
        server: enrollment.launch_owner.clone(),
        port: 49000,
        thread: enrollment.binding_id.clone(),
    };
    for replacement in [
        Record {
            generation: uuid::Uuid::new_v4().to_string(),
            ..enrollment.clone()
        },
        Record {
            launch_owner: Process::of(&ProcessIncarnation::new(42, "other-launch").unwrap()),
            ..enrollment.clone()
        },
        Record {
            launch_owner: Process::of(&ProcessIncarnation::new(43, "launch-start").unwrap()),
            ..enrollment.clone()
        },
    ] {
        store.write(&replacement).unwrap();
        assert!(!store.withdraw(&enrollment).unwrap());
        assert!(store.ready(&enrollment, ready.clone()).is_err());
        assert!(store.read(&enrollment.binding_id).unwrap() == Some(replacement));
    }
    // No liveness query participates in compare/remove. A replacement whose
    // owner is already gone receives exactly the same preservation guarantee.
}

#[test]
fn mutations_are_serialized_and_existing_enrollment_is_not_overwritten() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let enrollment = record();
    store
        .create(&enrollment, |_| RuntimeLiveness::Alive)
        .unwrap();
    assert_eq!(
        store
            .create(&enrollment, |_| RuntimeLiveness::Alive)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    let lock = store.lock(&enrollment.binding_id).unwrap();
    assert!(store.withdraw(&enrollment).is_err());
    drop(lock);
    assert!(store.read(&enrollment.binding_id).unwrap() == Some(enrollment.clone()));
    assert!(store.withdraw(&enrollment).unwrap());
}

#[test]
fn malformed_public_symlink_and_nonfile_records_fail_closed() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let enrollment = record();
    let path = store.path(&enrollment.binding_id).unwrap();
    crate::private_file::replace(&path, b"{").unwrap();
    assert!(store.read(&enrollment.binding_id).is_err());
    store.write(&enrollment).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.read(&enrollment.binding_id).is_err());
    fs::remove_file(&path).unwrap();
    let target = fixture.path.join("unrelated");
    fs::write(&target, b"untouched").unwrap();
    symlink(&target, &path).unwrap();
    assert!(store.read(&enrollment.binding_id).is_err());
    assert!(store.withdraw(&enrollment).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"untouched");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(store.read(&enrollment.binding_id).is_err());
}

#[test]
fn untrusted_directory_and_invalid_identifiers_are_not_admitted() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    assert!(store.read("../other").is_err());
    fs::set_permissions(&store.directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Store::open(&fixture.path).is_err());
    assert!(store.read(&record().binding_id).is_err());
}

#[test]
fn crashed_launch_can_be_replaced_but_stale_withdraw_cannot_remove_new_opt_in() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let previous = record();
    store.create(&previous, |_| RuntimeLiveness::Alive).unwrap();
    store
        .foreground(
            &previous,
            &ProcessIncarnation::new(44, "old-foreground").unwrap(),
        )
        .unwrap();
    let next = Record::new(
        &previous.binding_id,
        &ProcessIncarnation::new(43, "new-launch").unwrap(),
    )
    .unwrap();
    store
        .create(&next, |owner| {
            // Probes happen inside the same stable serialization domain as write.
            assert!(store.lock(&next.binding_id).is_err());
            if owner.pid() == 43 {
                RuntimeLiveness::Alive
            } else {
                RuntimeLiveness::Gone
            }
        })
        .unwrap();
    assert!(!store.withdraw(&previous).unwrap());
    assert!(store.read(&next.binding_id).unwrap() == Some(next.clone()));
    assert!(store.withdraw(&next).unwrap());
}

#[test]
fn old_absence_without_live_new_authority_or_unknown_old_owner_cannot_take_over() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let previous = record();
    store.create(&previous, |_| RuntimeLiveness::Alive).unwrap();
    let next = Record::new(
        &previous.binding_id,
        &ProcessIncarnation::new(43, "new-launch").unwrap(),
    )
    .unwrap();
    for (new, old) in [
        (RuntimeLiveness::Unknown, RuntimeLiveness::Gone),
        (RuntimeLiveness::Gone, RuntimeLiveness::Gone),
        (RuntimeLiveness::Alive, RuntimeLiveness::Unknown),
        (RuntimeLiveness::Alive, RuntimeLiveness::Alive),
    ] {
        assert!(
            store
                .create(&next, |owner| if owner.pid() == 43 { new } else { old })
                .is_err()
        );
        assert!(store.read(&previous.binding_id).unwrap() == Some(previous.clone()));
    }
}

fn attributed() -> Record {
    let mut record = record();
    record.attribution = Some(
        Attribution::new(
            "33333333-3333-4333-8333-333333333333",
            &ServerEvidence {
                host: tmt_core::host::HostKind::Tmux,
                server_id: "44444444-4444-4444-8444-444444444444".into(),
                socket_path: "/owned/tmux.sock".into(),
                server_pid: 7,
                server_start_time: "server-incarnation".into(),
            },
            "%2",
            8,
        )
        .unwrap(),
    );
    record
}
#[test]
fn unknown_relaunch_requires_same_address_and_every_recorded_process_gone() {
    for (server, same_pane, succeeds) in [
        (RuntimeLiveness::Alive, true, false),
        (RuntimeLiveness::Unknown, true, false),
        (RuntimeLiveness::Gone, false, false),
        (RuntimeLiveness::Gone, true, true),
    ] {
        let fixture = TestDirectory::new();
        let store = Store::open(&fixture.path).unwrap();
        let mut previous = attributed();
        previous.ready = Some(Ready {
            server: Process {
                pid: 44,
                start: "old-server".into(),
            },
            port: 49000,
            thread: previous.binding_id.clone(),
        });
        store.create(&previous, |_| RuntimeLiveness::Alive).unwrap();
        let mut next = previous.clone();
        next.generation = uuid::Uuid::new_v4().to_string();
        next.launch_owner = Process {
            pid: 43,
            start: "new-launch".into(),
        };
        next.ready = None;
        if !same_pane {
            next.attribution.as_mut().unwrap().pane_pid += 1;
        }
        let observe = |process: &ProcessIncarnation| match process.pid() {
            43 => RuntimeLiveness::Alive,
            44 => server,
            _ => RuntimeLiveness::Gone,
        };
        assert!(
            !previous.ended(&observe),
            "Unknown is never prunable, even with a gone ready server"
        );
        assert_eq!(store.create(&next, observe).is_ok(), succeeds);
        if succeeds {
            assert!(!store.withdraw(&previous).unwrap());
        } else {
            assert!(store.read(&previous.binding_id).unwrap() == Some(previous));
        }
    }
}
#[test]
fn foreground_publication_is_exact_lease_once_and_blocks_live_takeover() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let original = attributed();
    store.create(&original, |_| RuntimeLiveness::Alive).unwrap();
    let foreground = ProcessIncarnation::new(44, "foreground").unwrap();
    let mut replacement = original.clone();
    replacement.generation = uuid::Uuid::new_v4().to_string();
    assert!(store.foreground(&replacement, &foreground).is_err());
    store.foreground(&original, &foreground).unwrap();
    assert!(store.foreground(&original, &foreground).is_err());
    replacement.launch_owner = Process {
        pid: 43,
        start: "next".into(),
    };
    for status in [RuntimeLiveness::Alive, RuntimeLiveness::Unknown] {
        assert!(
            store
                .create(&replacement, |process| match process.pid() {
                    43 => RuntimeLiveness::Alive,
                    44 => status,
                    _ => RuntimeLiveness::Gone,
                })
                .is_err()
        );
    }
    let persisted = store.read(&original.binding_id).unwrap().unwrap();
    assert!(persisted.ended(&|_| RuntimeLiveness::Gone));
    assert!(!persisted.ended(&|process| if process.pid() == 44 {
        RuntimeLiveness::Alive
    } else {
        RuntimeLiveness::Gone
    }));
}
#[test]
fn attribution_checks_server_incarnation_pane_process_and_identity_shape() {
    let record = attributed();
    let attribution = record.attribution.unwrap();
    let mut server = ServerEvidence {
        host: tmt_core::host::HostKind::Tmux,
        server_id: attribution.server_id.clone(),
        socket_path: attribution.socket_path.clone(),
        server_pid: attribution.server.pid,
        server_start_time: attribution.server.start.clone(),
    };
    assert!(attribution.matches(&server, "%2", 8));
    assert!(!attribution.matches(&server, "%2", 9));
    server.server_start_time.push('x');
    assert!(!attribution.matches(&server, "%2", 8));
    assert!(Attribution::new("not-uuid", &server, "%2", 8).is_err());
    assert!(Attribution::new(&attribution.identity_id, &server, "", 8).is_err());
    assert!(Attribution::new(&attribution.identity_id, &server, "%2", 0).is_err());
}

#[test]
fn prune_removes_only_proven_ended_and_never_unknown_or_live_evidence() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let unknown = attributed();
    store.create(&unknown, |_| RuntimeLiveness::Alive).unwrap();
    let mut ended = attributed();
    ended.binding_id = "55555555-5555-4555-8555-555555555555".into();
    store.create(&ended, |_| RuntimeLiveness::Alive).unwrap();
    store
        .foreground(&ended, &ProcessIncarnation::new(44, "foreground").unwrap())
        .unwrap();
    assert_eq!(
        store
            .prune(
                Instant::now() + std::time::Duration::from_secs(1),
                |process| {
                    if process.pid() == 44 {
                        RuntimeLiveness::Unknown
                    } else {
                        RuntimeLiveness::Gone
                    }
                }
            )
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .prune(Instant::now() + std::time::Duration::from_secs(1), |_| {
                RuntimeLiveness::Gone
            })
            .unwrap(),
        1
    );
    assert!(store.read(&unknown.binding_id).unwrap() == Some(unknown));
    assert!(store.read(&ended.binding_id).unwrap().is_none());
}

#[test]
fn recovery_names_exact_pane_foreground_and_the_exact_enrollment_to_recover() {
    let mut record = attributed();
    record.foreground = Foreground::Known(Process {
        pid: 99,
        start: "original-start".into(),
    });
    let path = Path::new("/owned/it's private/record.json");
    let message = recovery(path, &record);
    assert!(message.contains("pane \"%2\" (pid 8)"));
    assert!(message.contains("original foreground pid 99 start \"original-start\""));
    assert!(message.contains("owned app-server are gone"));
    assert!(message.contains(&format!(
        "tmt channel recover --binding {} --generation {}",
        record.binding_id, record.generation
    )));
    assert!(message.contains("sends or pastes nothing"));
    assert!(!message.contains("rm --"));
}

#[test]
fn a_manual_removal_quotes_a_path_as_one_shell_word() {
    let path = Path::new("/owned/it's private/record.json");
    // The shell decodes one exact argument; this fixture never executes rm.
    let decoded = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            &format!("printf '%s' {}", shell_quote(path.to_str().unwrap())),
        ])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    assert_eq!(decoded.stdout, path.as_os_str().as_encoded_bytes());
}

#[test]
fn fresh_binding_is_immutable_and_ready_requires_exact_admitted_foreground() {
    let fixture = crate::test_support::TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let original = record();
    store.create(&original, |_| RuntimeLiveness::Alive).unwrap();
    let endpoint = FreshEndpoint {
        server: Process::of(&ProcessIncarnation::new(12, "server").unwrap()),
        port: 49000,
        cwd: "/tmp".into(),
        thread: None,
    };
    let pending = store.fresh_endpoint(&original, endpoint).unwrap();
    let foreground = ProcessIncarnation::new(13, "foreground").unwrap();
    let pending = store.foreground(&pending, &foreground).unwrap();
    let candidate = store
        .fresh_thread(&pending, "22222222-2222-4222-8222-222222222222")
        .unwrap();
    assert!(candidate.ready.is_none());
    assert!(
        store
            .fresh_thread(&candidate, "33333333-3333-4333-8333-333333333333")
            .is_err()
    );
    assert!(
        store
            .admit_fresh(
                &candidate,
                &ProcessIncarnation::new(14, "wrong").unwrap(),
                |_| RuntimeLiveness::Alive
            )
            .is_err()
    );
    assert!(
        store
            .admit_fresh(&candidate, &foreground, |_| RuntimeLiveness::Unknown)
            .is_err()
    );
    let unchanged = store.read(&candidate.binding_id).unwrap().unwrap();
    assert!(unchanged == candidate);
    let ready = store
        .admit_fresh(&candidate, &foreground, |_| RuntimeLiveness::Alive)
        .unwrap();
    assert_eq!(
        ready.ready.unwrap().thread,
        "22222222-2222-4222-8222-222222222222"
    );
    assert!(
        store
            .admit_fresh(&candidate, &foreground, |_| RuntimeLiveness::Alive)
            .is_err()
    );
}

#[test]
fn live_pruning_does_not_block_fresh_ready_publication() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let original = record();
    store.create(&original, |_| RuntimeLiveness::Alive).unwrap();
    let pending = store
        .fresh_endpoint(
            &original,
            FreshEndpoint {
                server: Process::of(&ProcessIncarnation::new(12, "server").unwrap()),
                port: 49000,
                cwd: "/tmp".into(),
                thread: None,
            },
        )
        .unwrap();
    let foreground = ProcessIncarnation::new(13, "foreground").unwrap();
    let pending = store.foreground(&pending, &foreground).unwrap();
    let candidate = store
        .fresh_thread(&pending, "22222222-2222-4222-8222-222222222222")
        .unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(0);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
    let publication = std::thread::scope(|scope| {
        let pruning_store = &store;
        let pruning = scope.spawn(move || {
            pruning_store.prune(Instant::now() + std::time::Duration::from_secs(5), |_| {
                entered_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                RuntimeLiveness::Alive
            })
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let publication = store.admit_fresh(&candidate, &foreground, |_| RuntimeLiveness::Alive);
        release_tx.send(()).unwrap();
        assert_eq!(pruning.join().unwrap().unwrap(), 0);
        publication
    });
    assert!(
        publication.is_ok(),
        "live-record liveness observation excluded Ready publication: {:?}",
        publication.as_ref().err()
    );
    assert_eq!(
        publication.unwrap().ready.unwrap().thread,
        "22222222-2222-4222-8222-222222222222"
    );
}

#[test]
fn pruning_preserves_a_replacement_published_during_snapshot_observation() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let original = record();
    store.create(&original, |_| RuntimeLiveness::Alive).unwrap();
    let original = store
        .foreground(
            &original,
            &ProcessIncarnation::new(44, "foreground").unwrap(),
        )
        .unwrap();
    let mut replacement = original.clone();
    replacement.generation = uuid::Uuid::new_v4().to_string();
    replacement.launch_owner = Process::of(&ProcessIncarnation::new(45, "new-owner").unwrap());
    replacement.foreground = Foreground::Unknown;
    let published = std::cell::Cell::new(false);
    assert_eq!(
        store
            .prune(Instant::now() + std::time::Duration::from_secs(5), |_| {
                if !published.replace(true) {
                    store
                        .create(&replacement, |process| {
                            if process.pid() == 45 {
                                RuntimeLiveness::Alive
                            } else {
                                RuntimeLiveness::Gone
                            }
                        })
                        .unwrap();
                }
                RuntimeLiveness::Gone
            })
            .unwrap(),
        0
    );
    assert!(store.read(&original.binding_id).unwrap() == Some(replacement));
}

#[test]
fn pruning_rechecks_ended_proof_under_lock_before_removal() {
    let fixture = TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let original = record();
    store.create(&original, |_| RuntimeLiveness::Alive).unwrap();
    let original = store
        .foreground(
            &original,
            &ProcessIncarnation::new(44, "foreground").unwrap(),
        )
        .unwrap();
    let probes = std::cell::Cell::new(0);
    assert_eq!(
        store
            .prune(Instant::now() + std::time::Duration::from_secs(5), |_| {
                let count = probes.get();
                probes.set(count + 1);
                if count < 2 {
                    assert!(store.lock(&original.binding_id).is_ok());
                    RuntimeLiveness::Gone
                } else {
                    assert!(store.lock(&original.binding_id).is_err());
                    RuntimeLiveness::Unknown
                }
            })
            .unwrap(),
        0
    );
    assert_eq!(probes.get(), 3);
    assert!(store.read(&original.binding_id).unwrap() == Some(original));
}
