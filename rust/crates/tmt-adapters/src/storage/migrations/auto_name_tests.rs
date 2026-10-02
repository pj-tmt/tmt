use super::*;

#[test]
fn existing_names_never_gain_automatic_provenance_and_the_flag_is_cursor_tracked() {
    let mut db = Connection::open_in_memory().unwrap();
    apply_through(&mut db, 42).unwrap();
    db.execute("INSERT INTO identities (id, name, canonical_name, lifetime, created_at, updated_at) VALUES ('fixture', 'claude-012345678abc', 'claude-012345678abc', 'temporary', 'created', 'updated')", []).unwrap();
    apply(&mut db).unwrap();
    let flag = || {
        db.query_row(
            "SELECT auto_named FROM identities WHERE id = 'fixture'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
    };
    let cursor = || {
        db.query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
    };
    assert_eq!(flag(), 0);
    let before = cursor();
    db.execute(
        "UPDATE identities SET auto_named = 1 WHERE id = 'fixture'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 1);
    assert_eq!(flag(), 1);
    db.execute(
        "UPDATE identities SET auto_named = 1 WHERE id = 'fixture'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 1, "unchanged provenance is not a change");
    assert!(
        db.execute(
            "UPDATE identities SET auto_named = 2 WHERE id = 'fixture'",
            []
        )
        .is_err()
    );
    assert_eq!(flag(), 1);
    db.execute(
        "UPDATE identities SET auto_named = 0 WHERE id = 'fixture'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 2);
}
