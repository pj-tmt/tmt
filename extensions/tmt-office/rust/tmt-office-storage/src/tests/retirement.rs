//! The pairing fence over Office storage, against disposable roots.
use super::{
    Root,
    selection::{ADA, switched},
};
use crate::retirement::OfficeRetirementFence;
use rusqlite::Connection;
use tmt_adapters::office_pairing::RetirementFence;

const UNKNOWN: &str = "99999999-9999-4999-8999-999999999999";

fn retire_in_core(root: &Root) {
    root.source()
        .execute(
            "UPDATE identities SET retired_at_ms = 5 WHERE id = ?",
            [ADA],
        )
        .unwrap();
}

fn markers(root: &Root) -> i64 {
    Connection::open(&root.layout.database)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM office_retired_identities",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn before_the_switch_core_retirement_fences_and_marking_changes_nothing() {
    let root = Root::new();
    root.source()
        .execute(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES (?, 'Ada', 'ada', 't', 't', 'saved')",
            [ADA],
        )
        .unwrap();
    let fence = OfficeRetirementFence::new(root.layout.clone());
    assert!(!fence.is_retired(ADA).unwrap());
    assert!(
        fence.is_retired(UNKNOWN).unwrap(),
        "unknown identities fail closed"
    );
    fence.mark_retired(ADA).unwrap();
    assert!(!fence.is_retired(ADA).unwrap(), "core stays authoritative");
    assert!(!root.layout.database.exists());
    retire_in_core(&root);
    assert!(fence.is_retired(ADA).unwrap());
}

#[test]
fn after_the_switch_the_office_marker_fences_idempotently() {
    let root = switched();
    let fence = OfficeRetirementFence::new(root.layout.clone());
    assert!(!fence.is_retired(ADA).unwrap());
    fence.mark_retired(ADA).unwrap();
    fence.mark_retired(ADA).unwrap();
    assert_eq!(markers(&root), 1);
    // Office's record fences even while core still reports the identity active.
    assert!(fence.is_retired(ADA).unwrap());
    let (retired, recorded): (i64, i64) = Connection::open(&root.layout.database)
        .unwrap()
        .query_row(
            "SELECT retired_at_ms, recorded_at_ms FROM office_retired_identities WHERE identity_id = ?",
            [ADA],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(retired > 0 && retired == recorded);
    // The marker is Office-local: the retained core copy has no such table.
    assert!(
        root.source()
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE name = 'office_retired_identities'",
                [],
                |_| Ok(())
            )
            .is_err()
    );
}

#[test]
fn after_the_switch_core_retirement_still_fences_without_a_marker() {
    let root = switched();
    retire_in_core(&root);
    let fence = OfficeRetirementFence::new(root.layout.clone());
    assert!(fence.is_retired(ADA).unwrap());
    assert_eq!(markers(&root), 0);
}

#[test]
fn an_unusable_store_blocks_rather_than_admits() {
    let root = switched();
    std::fs::remove_file(&root.layout.database).unwrap();
    let fence = OfficeRetirementFence::new(root.layout.clone());
    assert_eq!(
        fence.is_retired(ADA),
        Err(tmt_office_model::office_protocol::OfficeError::CredentialsUnavailable)
    );
}
