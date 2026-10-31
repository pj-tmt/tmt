//! Forward from the actual 086 predecessor; never edit a frozen archive.
use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
#[test]
fn canonical_jobs_migrate_after_plain_program_without_rewriting_existing_records() {
    let d = TestDirectory::new();
    let path = d.path.join("predecessor.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 86).unwrap();
    old.execute_batch("INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES('11111111-1111-4111-8111-111111111111','Member','member','t','t','saved');
        INSERT INTO orchestration_settings VALUES(1,3,'UTC');
        INSERT INTO orchestration_decisions(recorded_ms,expires_ms,subject_kind,subject_id,actor_id,recipient_id,action,outcome,reason,configuration_revision,time_zone,sources)
        VALUES(1000,10000,'identity','11111111-1111-4111-8111-111111111111',NULL,'11111111-1111-4111-8111-111111111111','send','allowed','existing decision',3,'UTC','[]');").unwrap();
    let bindings: String = old
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='bindings'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(bindings.contains("plain_program"));
    let cursor: i64 = old
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    old.close().unwrap();
    let mut migrated = Storage::open(&path).unwrap();
    assert_eq!(migrated.health().unwrap().schema_version, 87);
    migrated.close().unwrap();
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='bindings'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        bindings
    );
    assert_eq!(
        db.query_row(
            "SELECT revision,time_zone FROM orchestration_settings",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        )
        .unwrap(),
        (3, "UTC".into())
    );
    assert_eq!(
        db.query_row(
            "SELECT reason,recorded_ms,expires_ms FROM orchestration_decisions",
            [],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?
            ))
        )
        .unwrap(),
        ("existing decision".into(), 1000, 10000)
    );
    assert_eq!(
        db.query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        cursor,
        "empty job tables do not invent a data change"
    );
    for table in [
        "orchestration_jobs",
        "orchestration_job_legacy",
        "orchestration_job_legacy_counters",
        "orchestration_job_imports",
        "orchestration_job_runs",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(
        db.query_row("SELECT name FROM _migrations WHERE version=87", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "create canonical agent jobs and legacy import receipts"
    );
    assert_eq!(
        db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}
