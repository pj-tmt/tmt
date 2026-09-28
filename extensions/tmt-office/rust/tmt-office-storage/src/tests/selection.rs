//! Store selection by core's cutover receipt, against disposable roots.
use super::{Crash, Root, fault};
use crate::{
    OfficeStore,
    migration::{self, Quiesce, Quiesced},
};
use rusqlite::Connection;
use std::panic::{AssertUnwindSafe, catch_unwind};
use tmt_office_model::{
    office_block::LocalBlockTarget,
    office_board::{Actor, Category, OfficeBoardRepository, PostRequest, ShowRequest},
    office_whiteboard::document::{LOBBY_DOCUMENT, SaveDocument},
};

const ADA: &str = "11111111-1111-4111-8111-111111111111";
const OPERATION: &str = "22222222-2222-4222-8222-222222222222";
const FENCE: &str = "Office data moved to Office storage";

struct Stopped;

impl Quiesce for Stopped {
    fn quiesce(&self) -> Result<Quiesced<'_>, String> {
        Ok(Quiesced {
            was_running: false,
            guard: Box::new(()),
        })
    }
}

/// A root with one core identity whose Office rows moved to `office.db`.
fn switched() -> Root {
    let root = Root::new();
    root.source()
        .execute(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES (?, 'Ada', 'ada', 't', 't', 'saved')",
            [ADA],
        )
        .unwrap();
    migration::prepare(&root.layout).unwrap();
    migration::copy(&root.layout).unwrap();
    migration::verify(&root.layout).unwrap();
    migration::switch(&root.layout, &Stopped).unwrap();
    root
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

/// Rows per Office table written by the checks below.
const WRITTEN: &[&str] = &[
    "office_local_profiles",
    "office_local_worlds",
    "office_board_entries",
    "office_whiteboards",
    "office_prop_packs",
    "office_avatar_packs",
];

fn prop_pack() -> tmt_office_model::codec::office_prop::ValidatedPropPack {
    tmt_office_model::codec::office_prop::validate_pack(
        br##"{"formatVersion":1,"label":"Switched","credit":"Test","license":"MIT","palette":["#00000000","#ffffffff"],"props":[{"key":"lamp","label":"Lamp","footprint":{"width":1,"height":1},"pixels":["1"]}]}"##,
    )
    .unwrap()
}

fn avatar_pack() -> tmt_office_model::codec::office_avatar::ValidatedAvatarPack {
    tmt_office_model::codec::office_avatar::validate_pack(include_bytes!(
        "../../../../contracts/avatar-pack-v1-sample.tmtavatar.json"
    ))
    .unwrap()
}

/// One write and one read through every repository. The first world save
/// retires legacy blocks, so the block write is checked in `written` first.
fn exercise(store: &mut OfficeStore, written: &std::path::Path) {
    let profile = store.show_local_profile(ADA).unwrap();
    let applied = store.apply_local_profile(ADA, 0, &profile.profile).unwrap();
    assert!(applied.changed);
    assert!(store.show_local_profile(ADA).unwrap().exists);

    let lobby = store.show_local_block(&LocalBlockTarget::Lobby).unwrap();
    store
        .apply_local_block(&LocalBlockTarget::Lobby, 0, &lobby.layout)
        .unwrap();
    assert!(
        store
            .show_local_block(&LocalBlockTarget::Lobby)
            .unwrap()
            .exists()
    );
    assert_eq!(
        count(&Connection::open(written).unwrap(), "office_local_blocks"),
        1
    );

    let world = store.show_local_world().unwrap();
    let saved = store
        .apply_local_world(
            world.revision,
            world.legacy_basis.as_deref(),
            &world.layout,
            1_000,
        )
        .unwrap();
    assert_eq!(store.show_local_world().unwrap().revision, saved.revision);

    let posted = store
        .post(&PostRequest {
            category: Category::General,
            actor: Actor::Owner {
                world_id: saved.world_id.clone().expect("saved world"),
            },
            title: "Switched".into(),
            body: "Body".into(),
            operation_id: OPERATION.into(),
        })
        .unwrap();
    let shown = store
        .show(&ShowRequest {
            thread_id: posted.thread_id.clone(),
            reply_limit: 10,
            reply_cursor: None,
        })
        .unwrap();
    assert_eq!(shown.thread.id, posted.entry_id);

    let scene = tmt_office_model::codec::office_whiteboard::decode_scene(include_bytes!(
        "../../../../contracts/whiteboard-scene-v1.json"
    ))
    .unwrap();
    store
        .save_whiteboard(
            &SaveDocument {
                document_id: LOBBY_DOCUMENT.into(),
                expected_revision: 0,
                operation_id: OPERATION.into(),
                scene,
            },
            1_000,
        )
        .unwrap();
    assert_eq!(store.show_whiteboard(LOBBY_DOCUMENT).unwrap().revision, 1);

    let pack = prop_pack();
    store.install_local_prop_pack(0, &pack).unwrap();
    assert!(store.show_local_prop_pack(pack.digest()).is_ok());

    let avatars = avatar_pack();
    store.install_local_avatar_pack(0, &avatars).unwrap();
    assert!(store.show_local_avatar_pack(avatars.digest()).is_ok());
}

#[test]
fn before_the_switch_every_repository_uses_the_core_file() {
    let root = Root::new();
    root.source()
        .execute(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES (?, 'Ada', 'ada', 't', 't', 'saved')",
            [ADA],
        )
        .unwrap();
    let mut store = OfficeStore::open_configured(&root.layout).unwrap();
    exercise(&mut store, &root.layout.source);
    store.close().unwrap();
    for table in WRITTEN {
        assert!(count(&root.source(), table) > 0, "{table}");
    }
    assert!(!root.layout.database.exists());
}

#[test]
fn after_the_switch_every_repository_reads_and_writes_office_storage_only() {
    let root = switched();
    let core_before: Vec<i64> = WRITTEN
        .iter()
        .map(|table| count(&root.source(), table))
        .collect();
    let mut store = OfficeStore::open_configured(&root.layout).unwrap();
    exercise(&mut store, &root.layout.database);
    store.close().unwrap();
    assert_eq!(count(&root.source(), "office_local_blocks"), 0);
    let office = Connection::open(&root.layout.database).unwrap();
    for (table, before) in WRITTEN.iter().zip(core_before) {
        assert!(count(&office, table) > 0, "{table} in office.db");
        assert_eq!(count(&root.source(), table), before, "{table} in core");
    }
    // Reopening reads the same rows back.
    let reopened = OfficeStore::open_configured(&root.layout).unwrap();
    assert!(reopened.show_local_profile(ADA).unwrap().exists);
}

#[test]
fn a_stale_core_path_write_after_the_switch_fails_at_the_fence() {
    let root = switched();
    // The pre-switch layout, as an older companion would open it.
    let mut stale = OfficeStore::open(&root.layout.source).unwrap();
    let profile = stale.show_local_profile(ADA).unwrap();
    assert!(stale.apply_local_profile(ADA, 0, &profile.profile).is_err());
    assert!(stale.install_local_prop_pack(0, &prop_pack()).is_err());
    assert_eq!(count(&root.source(), "office_local_profiles"), 0);
    assert_eq!(count(&root.source(), "office_prop_packs"), 0);
    // The repositories report their generic storage error; the cause is the fence.
    let error = root
        .source()
        .execute(
            "INSERT INTO office_local_profiles (identity_id, revision, profile, updated_at_ms) VALUES (?, 1, '{}', 1)",
            [ADA],
        )
        .unwrap_err();
    assert!(error.to_string().contains(FENCE), "{error}");
}

#[test]
fn opening_finishes_an_interrupted_activation_and_reports_lost_storage() {
    let root = Root::new();
    migration::prepare(&root.layout).unwrap();
    migration::copy(&root.layout).unwrap();
    migration::verify(&root.layout).unwrap();
    fault::crash_at_point(Some("committed"));
    let crashed = catch_unwind(AssertUnwindSafe(|| {
        migration::switch(&root.layout, &Stopped)
    }));
    fault::crash_at_point(None);
    assert!(crashed.unwrap_err().downcast_ref::<Crash>().is_some());
    assert_eq!(super::state(&root), migration::State::Switching);
    OfficeStore::open_configured(&root.layout)
        .unwrap()
        .close()
        .unwrap();
    assert_eq!(super::state(&root), migration::State::Switched);

    std::fs::remove_file(&root.layout.database).unwrap();
    let error = OfficeStore::open_configured(&root.layout)
        .err()
        .expect("lost storage");
    assert!(error.to_string().contains("needs recovery"), "{error}");
    assert!(!root.layout.database.exists(), "never recreated empty");
}
