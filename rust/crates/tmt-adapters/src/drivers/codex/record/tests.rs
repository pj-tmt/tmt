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
