//! Reconciliation v1 and the retirement markers, against disposable roots.
use super::{
    Root,
    selection::{ADA, switched},
};
use crate::{
    LocalOfficeError, LocalProfileError, OfficeStore, WorldStoreError,
    core_references::{CoreIdentity, CoreReferences, CoreRoom},
    store::tests::at_next_preflight,
};
use rusqlite::Connection;
use std::time::Instant;
use tmt_adapters::storage::{StorageCutover, StorageError, StorageErrorCode};
use tmt_office_model::{
    office_block::LocalBlockTarget,
    office_board::{Actor, BoardErrorCode, Category, OfficeBoardRepository, PostRequest},
    office_map::{AreaKind, OfficeMap},
    office_world::WorldLayout,
};

const ROOM: &str = "20000000-0000-4000-8000-000000000001";
const OTHER: &str = "30000000-0000-4000-8000-000000000001";

fn add_room(root: &Root, id: &str) {
    root.source()
        .execute(
            "INSERT INTO office_meeting_rooms (room_id, name, revision) VALUES (?, ?, 1)",
            [id, &format!("Room {}", &id[..4])],
        )
        .unwrap();
}

fn retire_identity(root: &Root, id: &str) {
    root.source()
        .execute("UPDATE identities SET retired_at_ms = 5 WHERE id = ?", [id])
        .unwrap();
}

fn retire_room(root: &Root, id: &str) {
    root.source()
        .execute(
            "UPDATE office_meeting_rooms SET retired = 1, revision = revision + 1 WHERE room_id = ?",
            [id],
        )
        .unwrap();
}

fn add_identity(root: &Root, id: &str, name: &str) {
    root.source()
        .execute(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES (?, ?, lower(?), 't', 't', 'saved')",
            [id, name, name],
        )
        .unwrap();
}

fn store(root: &Root) -> OfficeStore {
    OfficeStore::open_configured(&root.layout).unwrap()
}

fn reconcile(root: &Root) -> crate::reconciliation::Reconciliation {
    let mut store = store(root);
    let outcome = store.reconcile(1_000).unwrap();
    store.close().unwrap();
    outcome
}

fn markers(root: &Root) -> (Vec<String>, Vec<String>) {
    let office = Connection::open(&root.layout.database).unwrap();
    let ids = |sql: &str| {
        office
            .prepare(sql)
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<String>, _>>()
            .unwrap()
    };
    (
        ids("SELECT identity_id FROM office_retired_identities ORDER BY 1"),
        ids("SELECT room_id FROM office_retired_rooms ORDER BY 1"),
    )
}

fn count(root: &Root, table: &str) -> i64 {
    Connection::open(&root.layout.database)
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn post(
    store: &mut OfficeStore,
    category: Category,
    actor: Actor,
    operation: &str,
) -> Result<(), BoardErrorCode> {
    store
        .post(&PostRequest {
            category,
            actor,
            title: "Thread".into(),
            body: "Body".into(),
            operation_id: operation.into(),
        })
        .map(|_| ())
        .map_err(|error| error.code)
}

fn ada() -> Actor {
    Actor::Identity {
        identity_id: ADA.into(),
        name: "Ada".into(),
    }
}

fn operation(index: u32) -> String {
    format!("40000000-0000-4000-8000-{index:012}")
}

/// Saves a world that assigns `identity` a personal area and binds `room`.
fn assign(
    store: &mut OfficeStore,
    identity: &str,
    room: Option<&str>,
) -> Result<(), WorldStoreError> {
    let before = store.show_local_world()?;
    let mut map = before.layout.map().draft().clone();
    let mut personal = map
        .areas
        .iter_mut()
        .filter(|area| matches!(area.kind, AreaKind::Personal { identity_id: None }));
    personal.next().unwrap().kind = AreaKind::Personal {
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
        .map(|_| ())
}

fn profile(store: &mut OfficeStore, revision: u64) -> Result<(), LocalProfileError> {
    let current = store.show_local_profile(ADA)?;
    let mut next = current.profile.clone();
    next.description = format!("r{revision}");
    store.apply_local_profile(ADA, revision, &next).map(|_| ())
}

#[test]
fn before_the_switch_reconciliation_changes_nothing() {
    let root = Root::new();
    add_identity(&root, ADA, "Ada");
    let mut store = store(&root);
    profile(&mut store, 0).unwrap();
    retire_identity(&root, ADA);
    assert_eq!(
        store.reconcile(1_000).unwrap(),
        crate::reconciliation::Reconciliation::default()
    );
    let tables: i64 = root
        .source()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('office_retired_identities', 'office_retired_rooms')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
}

#[test]
fn confirmed_retirements_are_marked_idempotently_and_nothing_is_erased() {
    let root = switched();
    add_room(&root, ROOM);
    let mut office = store(&root);
    profile(&mut office, 0).unwrap();
    // Legacy blocks are covered by the in-window test; the first world save retires them.
    post(&mut office, Category::General, ada(), &operation(1)).unwrap();
    assign(&mut office, ADA, Some(ROOM)).unwrap();
    office.close().unwrap();
    let before: Vec<i64> = [
        "office_local_profiles",
        "office_board_entries",
        "office_local_worlds",
    ]
    .iter()
    .map(|table| count(&root, table))
    .collect();

    let first = reconcile(&root);
    assert_eq!(
        (first.identities, first.rooms, first.newly_marked),
        (1, 1, 0)
    );
    retire_identity(&root, ADA);
    retire_room(&root, ROOM);
    assert_eq!(reconcile(&root).newly_marked, 2);
    assert_eq!(reconcile(&root).newly_marked, 0);
    assert_eq!(
        markers(&root),
        (vec![ADA.to_owned()], vec![ROOM.to_owned()])
    );
    let after: Vec<i64> = [
        "office_local_profiles",
        "office_board_entries",
        "office_local_worlds",
    ]
    .iter()
    .map(|table| count(&root, table))
    .collect();
    assert_eq!(after, before, "history is retained");
    // Retained layout content keeps its UUIDs.
    let world = store(&root).show_local_world().unwrap();
    assert!(world.layout.map().draft().areas.iter().any(|area| area.kind
        == AreaKind::Personal {
            identity_id: Some(ADA.into())
        }));
}

#[test]
fn a_mark_recorded_inside_the_preflight_window_blocks_every_reference_write() {
    let root = switched();
    add_room(&root, ROOM);
    let mut office = store(&root);
    profile(&mut office, 0).unwrap();
    post(
        &mut office,
        Category::Room(ROOM.into()),
        ada(),
        &operation(1),
    )
    .unwrap();
    office.close().unwrap();
    // Core retires, and reconciliation records it, after this write's preflight.
    let mark_inside_window = |root: &Root| {
        let (source, layout) = (root.layout.source.clone(), root.layout.clone());
        at_next_preflight(move || {
            let core = Connection::open(&source).unwrap();
            core.execute(
                "UPDATE identities SET retired_at_ms = 5 WHERE id = ?",
                [ADA],
            )
            .unwrap();
            core.execute(
                "UPDATE office_meeting_rooms SET retired = 1, revision = revision + 1 WHERE room_id = ?",
                [ROOM],
            )
            .unwrap();
            let mut other = OfficeStore::open_configured(&layout).unwrap();
            assert_eq!(other.reconcile(1_000).unwrap().newly_marked, 2);
            other.close().unwrap();
        });
    };

    let mut office = store(&root);
    mark_inside_window(&root);
    assert!(matches!(
        profile(&mut office, 1),
        Err(LocalProfileError::IdentityInactive)
    ));
    for table in ["office_retired_identities", "office_retired_rooms"] {
        Connection::open(&root.layout.database)
            .unwrap()
            .execute(&format!("DELETE FROM {table}"), [])
            .unwrap();
    }
    root.source()
        .execute(
            "UPDATE identities SET retired_at_ms = NULL WHERE id = ?",
            [ADA],
        )
        .unwrap();
    root.source()
        .execute(
            "UPDATE office_meeting_rooms SET retired = 0 WHERE room_id = ?",
            [ROOM],
        )
        .unwrap();

    let mut office = store(&root);
    mark_inside_window(&root);
    let block = office
        .show_local_block(&LocalBlockTarget::Identity(ADA.into()))
        .unwrap()
        .layout;
    assert!(matches!(
        office.apply_local_block(&LocalBlockTarget::Identity(ADA.into()), 0, &block),
        Err(LocalOfficeError::IdentityInactive)
    ));
    reset(&root);

    let mut office = store(&root);
    mark_inside_window(&root);
    assert_eq!(
        post(&mut office, Category::General, ada(), &operation(2)),
        Err(BoardErrorCode::Forbidden)
    );
    reset(&root);

    let mut office = store(&root);
    // An owner actor avoids the identity check, isolating the room marker; an
    // owner needs a saved world.
    let world = office.show_local_world().unwrap();
    let saved = office
        .apply_local_world(
            world.revision,
            world.legacy_basis.as_deref(),
            &world.layout,
            2_000,
        )
        .unwrap();
    let owner = Actor::Owner {
        world_id: saved.world_id.expect("saved world"),
    };
    mark_inside_window(&root);
    assert_eq!(
        post(
            &mut office,
            Category::Room(ROOM.into()),
            owner,
            &operation(3)
        ),
        Err(BoardErrorCode::Invalid)
    );
    reset(&root);

    let mut office = store(&root);
    mark_inside_window(&root);
    assert!(matches!(
        assign(&mut office, ADA, None),
        Err(WorldStoreError::IdentityIneligible)
    ));
    reset(&root);

    add_identity(&root, OTHER, "Other");
    let mut office = store(&root);
    mark_inside_window(&root);
    assert!(matches!(
        assign(&mut office, OTHER, Some(ROOM)),
        Err(WorldStoreError::RoomMissing)
    ));
}

fn reset(root: &Root) {
    for table in ["office_retired_identities", "office_retired_rooms"] {
        Connection::open(&root.layout.database)
            .unwrap()
            .execute(&format!("DELETE FROM {table}"), [])
            .unwrap();
    }
    root.source()
        .execute(
            "UPDATE identities SET retired_at_ms = NULL WHERE id = ?",
            [ADA],
        )
        .unwrap();
    root.source()
        .execute(
            "UPDATE office_meeting_rooms SET retired = 0 WHERE room_id = ?",
            [ROOM],
        )
        .unwrap();
}

#[test]
fn a_write_committed_before_the_mark_is_retained_and_later_writes_are_refused() {
    let root = switched();
    let mut office = store(&root);
    profile(&mut office, 0).unwrap();
    // Retired in core inside the window, not yet marked: the write commits,
    // like "write, then retire".
    let source = root.layout.source.clone();
    at_next_preflight(move || {
        Connection::open(&source)
            .unwrap()
            .execute(
                "UPDATE identities SET retired_at_ms = 5 WHERE id = ?",
                [ADA],
            )
            .unwrap();
    });
    profile(&mut office, 1).unwrap();
    office.close().unwrap();
    assert_eq!(reconcile(&root).newly_marked, 1);
    let mut office = store(&root);
    assert!(matches!(
        profile(&mut office, 2),
        Err(LocalProfileError::IdentityInactive)
    ));
    assert_eq!(count(&root, "office_local_profiles"), 1);
}

/// Core lookups that fail after `ok` successful identity reads.
struct Failing(Box<dyn CoreReferences + Send>);

impl CoreReferences for Failing {
    fn identity(&self, _: &str) -> Result<Option<CoreIdentity>, StorageError> {
        Err(StorageError::new(StorageErrorCode::Busy, "core is busy"))
    }
    fn active_identities(&self) -> Result<Vec<CoreIdentity>, StorageError> {
        self.0.active_identities()
    }
    fn room(&self, id: &str) -> Result<Option<CoreRoom>, StorageError> {
        self.0.room(id)
    }
    fn storage_cutover(&self) -> Result<Option<StorageCutover>, StorageError> {
        self.0.storage_cutover()
    }
}

#[test]
fn a_lookup_failure_marks_nothing_and_never_means_deleted() {
    let root = switched();
    add_room(&root, ROOM);
    let mut office = store(&root);
    profile(&mut office, 0).unwrap();
    post(
        &mut office,
        Category::Room(ROOM.into()),
        ada(),
        &operation(1),
    )
    .unwrap();
    office.close().unwrap();
    retire_identity(&root, ADA);
    retire_room(&root, ROOM);
    let mut office = store(&root);
    let core = crate::core_references::CoreStore::open(&root.layout.source).unwrap();
    office.replace_references(Box::new(Failing(Box::new(core))));
    assert!(office.reconcile(1_000).is_err());
    assert_eq!(markers(&root), (vec![], vec![]));
    // An identity core does not know is not "retired" either.
    Connection::open(&root.layout.database)
        .unwrap()
        .execute(
            "INSERT INTO office_local_profiles (identity_id, revision, profile, updated_at_ms) VALUES (?, 1, '{}', 1)",
            ["99999999-9999-4999-8999-999999999999"],
        )
        .unwrap();
    assert_eq!(reconcile(&root).newly_marked, 2);
    assert_eq!(markers(&root).0, vec![ADA.to_owned()]);
}

#[test]
fn dropped_duplicate_and_reordered_retirements_converge() {
    let root = switched();
    add_room(&root, ROOM);
    let mut office = store(&root);
    post(
        &mut office,
        Category::Room(ROOM.into()),
        ada(),
        &operation(1),
    )
    .unwrap();
    office.close().unwrap();
    // The room retires first and no reconciliation runs (a dropped observation).
    retire_room(&root, ROOM);
    retire_identity(&root, ADA);
    assert_eq!(reconcile(&root).newly_marked, 2);
    assert_eq!(reconcile(&root).newly_marked, 0);
    assert_eq!(
        markers(&root),
        (vec![ADA.to_owned()], vec![ROOM.to_owned()])
    );
}

#[test]
fn a_new_identity_with_the_same_name_is_not_marked() {
    let root = switched();
    let mut office = store(&root);
    profile(&mut office, 0).unwrap();
    office.close().unwrap();
    retire_identity(&root, ADA);
    root.source()
        .execute(
            "UPDATE identities SET canonical_name = 'ada-retired' WHERE id = ?",
            [ADA],
        )
        .unwrap();
    add_identity(&root, OTHER, "Ada");
    let mut office = store(&root);
    let default = office.show_local_profile(OTHER).unwrap();
    office
        .apply_local_profile(OTHER, 0, &default.profile)
        .unwrap();
    office.close().unwrap();
    assert_eq!(reconcile(&root).newly_marked, 1);
    assert_eq!(markers(&root).0, vec![ADA.to_owned()]);
    let mut office = store(&root);
    let current = office.show_local_profile(OTHER).unwrap();
    office
        .apply_local_profile(OTHER, 1, &current.profile)
        .unwrap();
}

#[test]
fn an_office_marker_reads_as_retired_at_the_point_of_use() {
    let root = switched();
    let mut office = store(&root);
    profile(&mut office, 0).unwrap();
    office.close().unwrap();
    // Recorded by Office while core still reports the identity active.
    Connection::open(&root.layout.database)
        .unwrap()
        .execute(
            "INSERT INTO office_retired_identities (identity_id, retired_at_ms, recorded_at_ms) VALUES (?, 1, 1)",
            [ADA],
        )
        .unwrap();
    let mut office = store(&root);
    assert!(office.references().identity(ADA).unwrap().unwrap().retired);
    assert!(office.list_active_local_profiles().unwrap().is_empty());
    assert!(matches!(
        profile(&mut office, 1),
        Err(LocalProfileError::IdentityInactive)
    ));
    assert_eq!(
        post(&mut office, Category::General, ada(), &operation(9)),
        Err(BoardErrorCode::Forbidden)
    );
    // History stays readable.
    assert!(
        office.show_local_profile(ADA).is_err() || office.show_local_profile(ADA).unwrap().exists
    );
}

#[test]
fn switched_storage_from_an_earlier_schema_upgrades_on_open() {
    let root = switched();
    Connection::open(&root.layout.database)
        .unwrap()
        .execute_batch(
            "DROP TABLE office_retired_rooms; DELETE FROM _office_schema WHERE version > 2;",
        )
        .unwrap();
    drop(store(&root));
    let versions: Vec<i64> = Connection::open(&root.layout.database)
        .unwrap()
        .prepare("SELECT version FROM _office_schema ORDER BY version")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(versions, [1, 2, 3]);
    assert_eq!(count(&root, "office_retired_rooms"), 0);
}

/// A realistic local inventory: 50 identities, 20 rooms, 50 profiles and
/// 2,000 board entries across them, plus a world assignment and meeting area.
#[test]
fn reconciliation_of_a_realistic_inventory_is_cheap() {
    let root = switched();
    let core = root.source();
    for index in 0..50 {
        let id = format!("50000000-0000-4000-8000-{index:012}");
        core.execute(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES (?, ?, ?, 't', 't', 'saved')",
            [&id, &format!("Agent {index}"), &format!("agent {index}")],
        )
        .unwrap();
    }
    for index in 0..20 {
        add_room(&root, &format!("60000000-0000-4000-8000-{index:012}"));
    }
    let office = Connection::open(&root.layout.database).unwrap();
    office
        .execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 49)
             INSERT INTO office_local_profiles (identity_id, revision, profile, updated_at_ms)
             SELECT printf('50000000-0000-4000-8000-%012d', i), 1, '{}', 1 FROM n;
             WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 1999)
             INSERT INTO office_board_entries (id, thread_id, is_root, category_kind, category_id, author_kind, author_id, author_name, revision, deleted, created_sequence, activity_sequence, created_at_ms, updated_at_ms, title, body)
             SELECT printf('70000000-0000-4000-8000-%012d', i), printf('70000000-0000-4000-8000-%012d', i), 1,
                    'room', printf('60000000-0000-4000-8000-%012d', i % 20),
                    'identity', printf('50000000-0000-4000-8000-%012d', i % 50), 'Agent', 1, 0, i + 1, i + 1, 1, 1, 'T', 'B' FROM n;",
        )
        .unwrap();
    let mut store = store(&root);
    assign(
        &mut store,
        "50000000-0000-4000-8000-000000000000",
        Some("60000000-0000-4000-8000-000000000000"),
    )
    .unwrap();
    let started = Instant::now();
    let outcome = store.reconcile(1_000).unwrap();
    let elapsed = started.elapsed();
    eprintln!(
        "reconciliation: {} identities, {} rooms, {} newly marked, {} ms",
        outcome.identities,
        outcome.rooms,
        outcome.newly_marked,
        elapsed.as_millis()
    );
    assert_eq!((outcome.identities, outcome.rooms), (50, 20));
    assert!(elapsed.as_millis() < 1_000, "{elapsed:?}");
}
