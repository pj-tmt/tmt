//! Office's read-only context summary, against disposable roots.
use super::{
    Root,
    selection::{ADA, switched},
    whole_source,
};
use crate::{OfficeStore, context};
use rusqlite::Connection;
use std::{collections::BTreeMap, fs, path::Path, time::SystemTime};
use tmt_office_model::{
    office_map::{AreaKind, OfficeMap},
    office_world::WorldLayout,
};

const ROOM: &str = "20000000-0000-4000-8000-000000000001";

fn assign(store: &mut OfficeStore, identity: &str, room: Option<&str>) {
    let before = store.show_local_world().unwrap();
    let mut map = before.layout.map().draft().clone();
    let mut personal = map
        .areas
        .iter_mut()
        .filter(|area| matches!(area.kind, AreaKind::Personal { identity_id: None }));
    let desk = personal.next().unwrap();
    desk.name = "North \"office\"".into();
    desk.kind = AreaKind::Personal {
        identity_id: Some(identity.into()),
    };
    if let Some(room) = room {
        personal.next().unwrap().kind = AreaKind::Meeting {
            room_id: room.into(),
        };
    }
    let layout = WorldLayout::new(
        OfficeMap::new(map).unwrap(),
        before.layout.objects().to_vec(),
    )
    .unwrap();
    store
        .apply_local_world(
            before.revision,
            before.legacy_basis.as_deref(),
            &layout,
            2_000,
        )
        .unwrap();
}

/// Every file with its size and mtime, except SQLite's own -wal/-shm sidecars,
/// which any reader of a WAL database may create (and which change their
/// directory's metadata, so directories are listed by name only). Database
/// contents are compared cell by cell separately.
fn tree(root: &Path) -> BTreeMap<String, (u64, SystemTime)> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with("-wal") || name.ends_with("-shm") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                pending.push(path.clone());
                entries.insert(path.display().to_string(), (0, SystemTime::UNIX_EPOCH));
                continue;
            }
            entries.insert(
                path.display().to_string(),
                (metadata.len(), metadata.modified().unwrap()),
            );
        }
    }
    entries
}

#[test]
fn an_assigned_identity_gets_a_short_summary_before_and_after_the_switch() {
    let root = Root::new();
    root.source()
        .execute(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES (?, 'Ada', 'ada', 't', 't', 'saved')",
            [ADA],
        )
        .unwrap();
    root.source()
        .execute(
            "INSERT INTO office_meeting_rooms (room_id, name, revision) VALUES (?, 'Review', 1)",
            [ROOM],
        )
        .unwrap();
    assert_eq!(
        context::summary(&root.layout, ADA),
        None,
        "no saved world yet"
    );
    let mut store = OfficeStore::open_configured(&root.layout).unwrap();
    assign(&mut store, ADA, Some(ROOM));
    store.close().unwrap();
    assert_eq!(
        context::summary(&root.layout, ADA).as_deref(),
        Some("Office: desk in \"North office\"; 1 meeting area.")
    );
    assert_eq!(
        context::summary(&root.layout, "99999999-9999-4999-8999-999999999999"),
        None
    );

    let switched = switched();
    let mut store = OfficeStore::open_configured(&switched.layout).unwrap();
    assign(&mut store, ADA, None);
    store.close().unwrap();
    assert_eq!(
        context::summary(&switched.layout, ADA).as_deref(),
        Some("Office: desk in \"North office\"; 0 meeting areas.")
    );
}

#[test]
fn the_summary_reads_without_creating_or_changing_anything() {
    let root = switched();
    let mut store = OfficeStore::open_configured(&root.layout).unwrap();
    assign(&mut store, ADA, None);
    store.close().unwrap();
    let files = tree(&root.path);
    let core = whole_source(&root.source());
    let office_before = whole_source(&Connection::open(&root.layout.database).unwrap());
    assert!(context::summary(&root.layout, ADA).is_some());
    assert_eq!(tree(&root.path), files);
    assert_eq!(whole_source(&root.source()), core);
    assert_eq!(
        whole_source(&Connection::open(&root.layout.database).unwrap()),
        office_before
    );
    // A missing store gives no summary and is not created.
    let empty = tempfile_root();
    assert_eq!(context::summary(&empty.layout, ADA), None);
    assert!(!empty.layout.database.exists());
}

fn tempfile_root() -> Root {
    let root = Root::new();
    fs::remove_file(&root.layout.source).unwrap();
    root
}
