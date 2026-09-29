//! First open of an install without user Office data switches through the
//! normal coordinator, against disposable roots.
use super::{Crash, Root, fault, legacy_user_data};
use crate::{
    OfficeStore,
    migration::{self, PlanState, State},
};
use rusqlite::Connection;
use std::panic::{AssertUnwindSafe, catch_unwind};
use tmt_adapters::storage::Storage;

const FENCE: &str = "Office data moved to Office storage";

fn receipt(root: &Root) -> bool {
    Storage::open(&root.layout.source)
        .unwrap()
        .extension_storage_cutover("office")
        .unwrap()
        .is_some()
}

#[test]
fn a_fresh_install_switches_on_first_open_with_a_receipt_and_no_backup() {
    let root = Root::new();
    let store = OfficeStore::open_configured(&root.layout).unwrap();
    assert!(store.is_office_database());
    drop(store);
    assert!(receipt(&root));
    assert_eq!(
        migration::status(&root.layout).unwrap().state,
        State::Switched
    );
    assert_eq!(
        migration::plan(&root.layout, None).unwrap().state,
        PlanState::Switched
    );
    assert!(!root.layout.staging.exists());
    assert!(!root.layout.backups.exists(), "nothing to back up");
    // The shared tables are fenced like any switched install.
    let error = root
        .source()
        .execute("UPDATE office_board_state SET revision = revision", [])
        .unwrap_err();
    assert!(error.to_string().contains(FENCE), "{error}");
    // Reopening uses the same store without switching again.
    assert!(
        OfficeStore::open_configured(&root.layout)
            .unwrap()
            .is_office_database()
    );
}

#[test]
fn an_install_with_user_office_data_keeps_the_legacy_store() {
    let root = Root::new();
    legacy_user_data(&root);
    assert!(
        !OfficeStore::open_configured(&root.layout)
            .unwrap()
            .is_office_database()
    );
    assert!(!root.layout.database.exists());
    assert!(!receipt(&root));
    assert_eq!(
        migration::plan(&root.layout, None).unwrap().state,
        PlanState::Pending
    );
}

#[test]
fn user_data_racing_in_before_the_decision_aborts_and_keeps_the_legacy_store() {
    let root = Root::new();
    let source = root.layout.source.clone();
    fault::at_point("fresh-verified", move || {
        Connection::open(source)
            .unwrap()
            .execute_batch(
                "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime)
                   VALUES ('88888888-8888-4888-8888-888888888888', 'Late', 'late', 't', 't', 'saved');
                 INSERT INTO office_local_profiles (identity_id, revision, profile, updated_at_ms)
                   VALUES ('88888888-8888-4888-8888-888888888888', 1, '{}', 1);",
            )
            .unwrap();
    });
    let store = OfficeStore::open_configured(&root.layout).unwrap();
    assert!(!store.is_office_database());
    assert!(!root.layout.database.exists());
    assert!(!receipt(&root));
    // The late row is still in the shared store, untouched.
    let rows: i64 = root
        .source()
        .query_row("SELECT count(*) FROM office_local_profiles", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn a_crash_after_the_fresh_decision_is_activated_by_the_next_open() {
    let root = Root::new();
    fault::crash_at_point(Some("fresh-committed"));
    let crashed = catch_unwind(AssertUnwindSafe(|| {
        OfficeStore::open_configured(&root.layout).map(|_| ())
    }));
    fault::crash_at_point(None);
    assert!(crashed.unwrap_err().downcast_ref::<Crash>().is_some());
    assert!(receipt(&root));
    assert_eq!(
        migration::status(&root.layout).unwrap().state,
        State::Switching
    );
    assert!(
        OfficeStore::open_configured(&root.layout)
            .unwrap()
            .is_office_database()
    );
    assert_eq!(
        migration::status(&root.layout).unwrap().state,
        State::Switched
    );
}

#[test]
fn concurrent_first_opens_switch_once_and_both_use_office_storage() {
    let root = Root::new();
    let layout = root.layout.clone();
    let opened: Vec<bool> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let layout = layout.clone();
                scope.spawn(move || {
                    OfficeStore::open_configured(&layout)
                        .unwrap()
                        .is_office_database()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    assert_eq!(opened, vec![true; 4]);
    assert!(receipt(&root));
    let receipts: i64 = root
        .source()
        .query_row(
            "SELECT count(*) FROM extension_storage_cutovers",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 1);
    assert!(!root.layout.staging.exists());
    assert_eq!(
        migration::status(&root.layout).unwrap().state,
        State::Switched
    );
}
