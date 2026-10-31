use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use tmt_core::binding::session::launch::SessionLearningDetail;

const BINDING: &str = "INSERT INTO identities (id,name,canonical_name,created_at,updated_at)
    VALUES ('identity','Alpha','alpha','created','updated');
    INSERT INTO bindings (id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at)
    VALUES ('binding','identity','tmux','%3','server','/tmp/private',41,'server-start',51,'bound','verified');";

const NO_ADMITTED_START: &str = "stopped:turn:Stop:no_admitted_start";

#[test]
fn the_missing_start_detail_preserves_schema_81_rows_and_change_cursor_triggers() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema81.db");
    let mut old = Connection::open(&path).unwrap();
    old.pragma_update(None, "foreign_keys", true).unwrap();
    apply_through(&mut old, 81).unwrap();
    old.execute_batch(BINDING).unwrap();
    let kept: Vec<&str> = include_str!("../schema/082.sql")
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix('\'')
                .and_then(|token| token.split('\'').next())
        })
        .filter(|token| *token != NO_ADMITTED_START)
        .collect();
    assert_eq!(kept.len(), 55);
    for (index, token) in kept.iter().enumerate() {
        old.execute(&observation(100 + index as u32, &format!("'{token}'")), [])
            .unwrap();
    }
    old.execute(&observation(200, "NULL"), []).unwrap();
    assert!(
        old.execute(&observation(201, &format!("'{NO_ADMITTED_START}'")), [])
            .is_err()
    );
    let cursor = |db: &Connection| {
        db.query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
    };
    let original_cursor = cursor(&old);
    let objects = |db: &Connection| {
        db.prepare("SELECT name FROM sqlite_schema WHERE tbl_name='launch_observations' AND type IN ('index','trigger') ORDER BY name")
            .unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    let original_objects = objects(&old);
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 87);
    storage.close().unwrap();
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let stored: Vec<Option<String>> = db
        .prepare("SELECT detail FROM launch_observations ORDER BY owner_pid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let expected: Vec<Option<String>> = kept
        .iter()
        .map(|token| Some((*token).to_owned()))
        .chain([None])
        .collect();
    assert_eq!(stored, expected, "all 55 prior tokens and NULL survive");
    assert_eq!(objects(&db), original_objects);
    assert_eq!(
        cursor(&db),
        original_cursor,
        "copying the column invents no row change"
    );
    db.execute(&observation(201, &format!("'{NO_ADMITTED_START}'")), [])
        .unwrap();
    for token in [
        "stopped:turn:Stop:other",
        "stopped:turn:Interrupt:no_admitted_start",
        "stopped:unknown:no_admitted_start",
    ] {
        assert!(
            db.execute(&observation(202, &format!("'{token}'")), [])
                .is_err(),
            "{token}"
        );
    }
    let before = cursor(&db);
    db.execute(
        "UPDATE launch_observations SET detail=? WHERE owner_pid=200",
        [NO_ADMITTED_START],
    )
    .unwrap();
    assert_eq!(cursor(&db), before + 1);
    assert!(
        db.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}

fn observation(owner_pid: u32, detail: &str) -> String {
    format!(
        "INSERT INTO launch_observations
            (binding_id,preferences_fingerprint,owner_pid,owner_start,driver,created_at_ms,expires_at_ms,step,reason,detail)
         VALUES ('binding','0000000000000000000000000000000000000000000000000000000000000000',{owner_pid},'owner-start','claude',100,200,'decode','not_admitted',{detail})"
    )
}

#[test]
fn the_widened_detail_list_keeps_every_stored_detail_and_accepts_the_new_tokens() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema76.db");
    let mut old = Connection::open(&path).unwrap();
    old.pragma_update(None, "foreign_keys", true).unwrap();
    apply_through(&mut old, 76).unwrap();
    old.execute_batch(BINDING).unwrap();
    // One row per token the previous list holds, and one without a detail.
    let kept: Vec<&str> = SessionLearningDetail::ALL
        .iter()
        .map(|detail| detail.as_str())
        .filter(|token| !token.starts_with("stopped:") && !token.starts_with("digest_stop:"))
        .collect();
    assert_eq!(kept.len(), 10);
    for (index, token) in kept.iter().enumerate() {
        old.execute(&observation(100 + index as u32, &format!("'{token}'")), [])
            .unwrap();
    }
    old.execute(&observation(200, "NULL"), []).unwrap();
    assert!(
        old.execute(&observation(201, "'stopped:turn:Stop:unbound_session'"), [])
            .is_err(),
        "schema 76 does not know the new tokens"
    );
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 87);
    storage.close().unwrap();

    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let stored: Vec<Option<String>> = db
        .prepare("SELECT detail FROM launch_observations ORDER BY owner_pid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let expected: Vec<Option<String>> = kept
        .iter()
        .map(|token| Some((*token).to_owned()))
        .chain([None])
        .collect();
    assert_eq!(
        stored, expected,
        "every stored detail survives the migration"
    );

    // Every token the Rust side can write is accepted, and nothing else.
    for (index, detail) in SessionLearningDetail::ALL.iter().enumerate() {
        db.execute(
            &observation(1_000 + index as u32, &format!("'{}'", detail.as_str())),
            [],
        )
        .unwrap_or_else(|error| panic!("{detail:?} was refused: {error}"));
    }
    for refused in [
        "stopped:turn:Stop:other",
        "stopped:invalid",
        "refused:turn",
        "",
    ] {
        assert!(
            db.execute(&observation(5_000, &format!("'{refused}'")), [])
                .is_err(),
            "{refused:?} must not be storable"
        );
    }
    // The change feed still moves when only the detail changes.
    let before: i64 = db
        .query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    db.execute(
        "UPDATE launch_observations SET detail = 'host_unsupported' WHERE owner_pid = 200",
        [],
    )
    .unwrap();
    let after: i64 = db
        .query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(after, before + 1);
}

#[test]
fn the_digest_stop_list_keeps_every_stored_detail_and_accepts_its_tokens() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema79.db");
    let mut old = Connection::open(&path).unwrap();
    old.pragma_update(None, "foreign_keys", true).unwrap();
    apply_through(&mut old, 79).unwrap();
    old.execute_batch(BINDING).unwrap();
    // One row per token 077 lists, and one without a detail.
    let kept: Vec<&str> = SessionLearningDetail::ALL
        .iter()
        .map(|detail| detail.as_str())
        .filter(|token| !token.starts_with("digest_stop:") && *token != NO_ADMITTED_START)
        .collect();
    assert_eq!(kept.len(), 34);
    for (index, token) in kept.iter().enumerate() {
        old.execute(&observation(100 + index as u32, &format!("'{token}'")), [])
            .unwrap();
    }
    old.execute(&observation(200, "NULL"), []).unwrap();
    assert!(
        old.execute(&observation(201, "'digest_stop:launch:owner'"), [])
            .is_err(),
        "schema 79 does not know the new tokens"
    );
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 87);
    storage.close().unwrap();

    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let stored: Vec<Option<String>> = db
        .prepare("SELECT detail FROM launch_observations ORDER BY owner_pid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let expected: Vec<Option<String>> = kept
        .iter()
        .map(|token| Some((*token).to_owned()))
        .chain([None])
        .collect();
    assert_eq!(
        stored, expected,
        "every stored detail survives the migration"
    );
    let digest: Vec<&str> = SessionLearningDetail::ALL
        .iter()
        .map(|detail| detail.as_str())
        .filter(|token| token.starts_with("digest_stop:"))
        .collect();
    assert_eq!(digest.len(), 55);
    for (index, token) in digest.iter().enumerate() {
        db.execute(
            &observation(1_000 + index as u32, &format!("'{token}'")),
            [],
        )
        .unwrap_or_else(|error| panic!("{token} was refused: {error}"));
    }
    for refused in [
        "digest_stop:nothing_fitting",
        "digest_stop:no_claim",
        "digest_stop:launch",
        "digest_stop:",
    ] {
        assert!(
            db.execute(&observation(5_000, &format!("'{refused}'")), [])
                .is_err(),
            "{refused:?} must not be storable"
        );
    }
    // The change feed still moves when only the detail changes.
    let before: i64 = db
        .query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    db.execute(
        "UPDATE launch_observations SET detail = 'digest_stop:worker_error' WHERE owner_pid = 200",
        [],
    )
    .unwrap();
    let after: i64 = db
        .query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(after, before + 1);
}

#[test]
fn diagnostic_stages_preserve_schema_84_rows_and_reject_unbounded_details() {
    let directory = TestDirectory::new();
    // Applying schema 84 directly also provisions its owner-only hook queue.
    let root = directory.path.join("store");
    crate::private_file::ensure_private_directory(&root).unwrap();
    let path = root.join("schema84.db");
    let mut old = Connection::open(&path).unwrap();
    old.pragma_update(None, "foreign_keys", true).unwrap();
    apply_through(&mut old, 84).unwrap();
    old.execute_batch(BINDING).unwrap();
    let previous: Vec<&str> = include_str!("../schema/082.sql")
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix('\'')
                .and_then(|token| token.split('\'').next())
        })
        .collect();
    assert_eq!(previous.len(), 56);
    for (index, token) in previous.iter().enumerate() {
        old.execute(&observation(100 + index as u32, &format!("'{token}'")), [])
            .unwrap();
    }
    old.execute(&observation(500, "NULL"), []).unwrap();
    assert!(
        old.execute(&observation(501, "'digest_stop:worker:open:busy'"), [])
            .is_err()
    );
    let cursor: i64 = old
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let objects: Vec<String> = old.prepare("SELECT name FROM sqlite_schema WHERE tbl_name='launch_observations' AND type IN ('index','trigger') ORDER BY name").unwrap()
        .query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
    old.close().unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 87);
    storage.close().unwrap();
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let stored: Vec<Option<String>> = db
        .prepare("SELECT detail FROM launch_observations ORDER BY owner_pid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        stored,
        previous
            .iter()
            .map(|token| Some((*token).to_owned()))
            .chain([None])
            .collect::<Vec<_>>()
    );
    let after: i64 = db
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        after, cursor,
        "constraint migration invents no conversation change"
    );
    let after_objects: Vec<String> = db.prepare("SELECT name FROM sqlite_schema WHERE tbl_name='launch_observations' AND type IN ('index','trigger') ORDER BY name").unwrap()
        .query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(objects, after_objects);
    for (index, detail) in SessionLearningDetail::ALL.iter().enumerate() {
        db.execute(
            &observation(1_000 + index as u32, &format!("'{}'", detail.as_str())),
            [],
        )
        .unwrap();
    }
    for invalid in [
        "digest_stop:worker:open:busy:payload",
        "digest_stop:launch:caller:argv",
        "digest_stop:worker_io\n",
    ] {
        assert!(
            db.execute(&observation(9_000, &format!("'{invalid}'")), [])
                .is_err()
        );
    }
    let before: i64 = db
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    db.execute(
        "UPDATE launch_observations SET detail='digest_stop:worker:open:busy' WHERE owner_pid=500",
        [],
    )
    .unwrap();
    let after: i64 = db
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        after,
        before + 1,
        "detail-only updates still publish one change"
    );
}
