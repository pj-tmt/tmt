use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use tmt_core::binding::session::launch::SessionLearningDetail;

const BINDING: &str = "INSERT INTO identities (id,name,canonical_name,created_at,updated_at)
    VALUES ('identity','Alpha','alpha','created','updated');
    INSERT INTO bindings (id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at)
    VALUES ('binding','identity','tmux','%3','server','/tmp/private',41,'server-start',51,'bound','verified');";

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
        .filter(|token| !token.starts_with("stopped:"))
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
    assert_eq!(storage.health().unwrap().schema_version, 77);
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
