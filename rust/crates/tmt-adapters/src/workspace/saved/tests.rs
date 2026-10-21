use super::*;
use crate::{
    test_support::TestDirectory,
    workspace::{
        publish_event,
        tests::{paths, sample},
    },
};
use std::os::unix::fs::MetadataExt;

fn fixture() -> (TestDirectory, ConfigPaths, String) {
    let root = TestDirectory::new();
    let paths = paths(&root.path);
    let mut snapshot = sample();
    snapshot.server.socket = root.path.join("socket").display().to_string();
    let socket = snapshot.server.socket.clone();
    publish_event(&paths, &socket, || Ok(snapshot)).unwrap();
    (root, paths, socket)
}

#[test]
fn generations_select_the_protected_layout_and_require_pinned_smaller_consent() {
    let root = TestDirectory::new();
    let paths = paths(&root.path);
    let mut rich = sample();
    rich.panes[0].identity = Some(tmt_core::workspace::WorkspaceIdentity {
        id: "f8c8b6c0-3adc-452c-9a69-000000000001".into(),
        name: "worker".into(),
        lifetime: "saved".into(),
        binding: "binding".into(),
        harness: Some("claude".into()),
        session: Some("session".into()),
        mode: Some("default".into()),
        channel: None,
    });
    for index in 2..=4 {
        let mut pane = rich.panes[0].clone();
        pane.id = format!("%{index}");
        pane.index = index - 1;
        pane.identity.as_mut().unwrap().id = format!("f8c8b6c0-3adc-452c-9a69-{index:012}");
        rich.panes.push(pane);
    }
    let socket = rich.server.socket.clone();
    publish_event(&paths, &socket, || Ok(rich.clone())).unwrap();
    let mut small = rich.clone();
    small.panes.truncate(1);
    small.captured_at_ms = 2;
    small.server.process = tmt_core::endpoint::ProcessIncarnation::new(11, "restart").unwrap();
    publish_event(&paths, &socket, || Ok(small.clone())).unwrap();
    let chosen = acquire_selected(&paths, &socket, None, None).unwrap();
    assert_eq!(chosen.snapshot, rich);
    assert!(chosen.protected);
    assert!(!chosen.requires_confirmation);
    let entries = list_snapshots(&paths, &socket).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!((entries[0].panes, entries[0].agents), (1, 1));
    assert!(!entries[0].protected);
    assert_eq!((entries[1].panes, entries[1].agents), (4, 4));
    assert!(entries[1].protected);
    let selector: SnapshotSelector = entries[0]
        .generation
        .as_ref()
        .unwrap()
        .id
        .as_str()
        .parse()
        .unwrap();
    assert!(selector.as_str().parse::<SnapshotName>().is_err());
    let explicit = acquire_selected(&paths, &socket, Some(&selector), None).unwrap();
    assert_eq!(explicit.snapshot, small);
    assert!(explicit.requires_confirmation);
    assert!(matches!(
        acquire_selected(&paths, &socket, Some(&selector), Some(&chosen.digest)),
        Err(Error::Changed)
    ));
    let pinned =
        acquire_selected(&paths, &socket, Some(&selector), Some(&explicit.digest)).unwrap();
    assert!(!pinned.requires_confirmation);
    assert_eq!(pinned.bytes, explicit.bytes);
    let id = pinned.generation.as_ref().unwrap().id.clone();
    assert!(
        !accept_generation(&paths, &socket, &id, None).unwrap(),
        "stale baseline cannot win"
    );
    assert_eq!(
        acquire_selected(&paths, &socket, None, None)
            .unwrap()
            .snapshot,
        rich
    );
    assert!(
        accept_generation(
            &paths,
            &socket,
            &id,
            pinned.baseline.as_ref().map(|generation| &generation.id)
        )
        .unwrap()
    );
    assert_eq!(
        acquire_selected(&paths, &socket, None, None)
            .unwrap()
            .snapshot,
        small
    );
}

#[test]
fn named_copy_is_exact_private_and_independent_of_automatic_latest() {
    let (_root, paths, socket) = fixture();
    let name = "work".parse().unwrap();
    let latest = destination(&paths.workspace_directory(&socket), None);
    let mut bytes = fs::read(&latest).unwrap();
    // A byte digest must not normalize valid whitespace through the codec.
    bytes.push(b'\n');
    fs::write(&latest, &bytes).unwrap();
    let entry = save(&paths, &socket, &name).unwrap();
    let saved = destination(&paths.workspace_directory(&socket), Some(&name));
    assert_eq!(fs::read(&saved).unwrap(), bytes);
    assert_eq!(entry.digest, sha256(&bytes));
    assert_eq!(fs::metadata(&saved).unwrap().mode() & 0o777, 0o600);
    let mut newer = decode(&bytes).unwrap();
    newer.captured_at_ms += 1;
    newer.windows[0].name = "changed layout".into();
    publish_event(&paths, &socket, || Ok(newer)).unwrap();
    assert_eq!(fs::read(&saved).unwrap(), bytes);
    assert_ne!(fs::read(&latest).unwrap(), bytes);
    assert!(matches!(save(&paths, &socket, &name), Err(Error::Exists)));
    let rows = list_snapshots(&paths, &socket).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows[0].name.is_none());
    assert_eq!(
        rows.iter()
            .find(|row| row.name.as_ref() == Some(&name))
            .unwrap()
            .digest,
        entry.digest
    );
    let other_socket = format!("{socket}-other");
    assert!(list_snapshots(&paths, &other_socket).unwrap().is_empty());
    assert!(matches!(
        acquire(&paths, &other_socket, Some(&name), None),
        Err(Error::NotFound)
    ));
    remove(&paths, &socket, &name).unwrap();
    assert!(matches!(
        remove(&paths, &socket, &name),
        Err(Error::NotFound)
    ));
    assert_eq!(save(&paths, &socket, &name).unwrap().digest, rows[0].digest);
}

#[test]
fn matching_guard_accepts_and_changed_bytes_refuse_without_reopening_acquired_input() {
    let (_root, paths, socket) = fixture();
    let original = acquire(&paths, &socket, None, None).unwrap();
    let frozen = acquire(&paths, &socket, None, Some(&original.digest)).unwrap();
    let mut next = original.snapshot.clone();
    next.captured_at_ms += 1;
    next.windows[0].name = "changed layout".into();
    publish_event(&paths, &socket, || Ok(next)).unwrap();
    assert!(matches!(
        acquire(&paths, &socket, None, Some(&original.digest)),
        Err(Error::Changed)
    ));
    assert_eq!(frozen.snapshot, original.snapshot);
    assert_eq!(frozen.digest, original.digest);
    assert_eq!(frozen.bytes, original.bytes);
}

#[test]
fn cap_and_nonblocking_lock_serialize_competing_saves_without_partial_files() {
    let (_root, paths, socket) = fixture();
    for index in 0..MAX_SAVED_SNAPSHOTS - 1 {
        save(&paths, &socket, &format!("item-{index}").parse().unwrap()).unwrap();
    }
    let directory = paths.workspace_directory(&socket);
    let held = lock(&directory).unwrap();
    let name = "busy".parse().unwrap();
    assert!(matches!(save(&paths, &socket, &name), Err(Error::Busy)));
    assert!(matches!(
        remove(&paths, &socket, &"item-0".parse().unwrap()),
        Err(Error::Busy)
    ));
    assert!(
        !publish_event(&paths, &socket, || panic!(
            "contention skips optional capture"
        ))
        .unwrap()
    );
    drop(held);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let writers = ["last-a", "last-b"].map(|name| {
        let paths = paths.clone();
        let socket = socket.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            save(&paths, &socket, &name.parse().unwrap())
        })
    });
    barrier.wait();
    let outcomes = writers.map(|thread| thread.join().unwrap());
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        outcomes
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|error| matches!(error, Error::Busy | Error::Limit))
    );
    assert!(matches!(save(&paths, &socket, &name), Err(Error::Limit)));
    assert_eq!(
        list_snapshots(&paths, &socket).unwrap().len(),
        MAX_SAVED_SNAPSHOTS + 1
    );
    assert!(fs::read_dir(&directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
    remove(&paths, &socket, &"item-0".parse().unwrap()).unwrap();
    save(&paths, &socket, &name).unwrap();
}

#[test]
fn no_clobber_preserves_foreign_destination_and_unowned_stage() {
    let root = TestDirectory::new();
    let destination = root.path.join("saved-work.json");
    fs::write(&destination, b"existing").unwrap();
    assert!(matches!(
        publish_new(&destination, b"new"),
        Err(Error::Exists)
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"existing");
    let stage = root
        .path
        .join(format!(".saved-work.json.{}.tmp", std::process::id()));
    assert!(!stage.exists());
    fs::write(&stage, b"unowned").unwrap();
    assert!(publish_new(&destination, b"new").is_err());
    assert_eq!(fs::read(&stage).unwrap(), b"unowned");
    assert_eq!(fs::read(&destination).unwrap(), b"existing");
}

#[test]
fn malformed_oversized_symlinked_and_wrong_socket_documents_are_preserved() {
    let (root, paths, socket) = fixture();
    let name = "work".parse().unwrap();
    let latest = destination(&paths.workspace_directory(&socket), None);
    let original = fs::read(&latest).unwrap();
    for bytes in [b"{\"version\":999}".to_vec(), vec![b' '; MAX_BYTES + 1]] {
        fs::write(&latest, &bytes).unwrap();
        assert!(save(&paths, &socket, &name).is_err());
        assert_eq!(fs::read(&latest).unwrap(), bytes);
        assert!(!destination(&paths.workspace_directory(&socket), Some(&name)).exists());
    }
    fs::remove_file(&latest).unwrap();
    let foreign = root.path.join("foreign");
    fs::write(&foreign, &original).unwrap();
    std::os::unix::fs::symlink(&foreign, &latest).unwrap();
    assert!(save(&paths, &socket, &name).is_err());
    assert!(fs::symlink_metadata(&latest).unwrap().is_symlink());
    assert_eq!(fs::read(&foreign).unwrap(), original);
    fs::remove_file(&latest).unwrap();
    let mut wrong = decode(&original).unwrap();
    wrong.server.socket.push_str("-different");
    fs::write(&latest, super::super::encode(&wrong).unwrap()).unwrap();
    assert!(acquire(&paths, &socket, None, None).is_err());
    fs::write(&latest, &original).unwrap();
    save(&paths, &socket, &name).unwrap();
    let named = destination(&paths.workspace_directory(&socket), Some(&name));
    fs::remove_file(&named).unwrap();
    std::os::unix::fs::symlink(&foreign, &named).unwrap();
    assert!(remove(&paths, &socket, &name).is_err());
    assert!(fs::symlink_metadata(&named).unwrap().is_symlink());
    assert_eq!(fs::read(&foreign).unwrap(), original);
}
