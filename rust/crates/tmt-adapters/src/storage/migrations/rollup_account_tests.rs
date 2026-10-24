use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

const ORIGINAL: &str = "identity_id,from_ms,input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous,sampled_at_ms,limits,details";

fn original_rows(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT quote(identity_id)||quote(from_ms)||quote(input_tokens)||quote(output_tokens)||quote(cached_input_tokens)||quote(covered_ms)||quote(complete)||quote(gap)||quote(discontinuous)||quote(sampled_at_ms)||quote(limits)||quote(details) FROM consumption_tier5m ORDER BY from_ms",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn cursor(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn the_rollup_account_columns_are_additive_and_leave_every_existing_row_alone() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema55.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 55).unwrap();
    old.execute_batch(&format!(
        "INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES('meter','Meter','meter','t','t','saved');
         INSERT INTO consumption_tier5m({ORIGINAL}) VALUES
         ('meter',0,10,5,2,300000,1,0,0,299000,NULL,NULL),
         ('meter',300000,20,10,4,150000,0,1,0,450000,'{{\"observedAtMs\":1}}','{{}}');
         CREATE TRIGGER refuse_account_columns BEFORE INSERT ON _migrations WHEN NEW.version=56
         BEGIN SELECT RAISE(ABORT,'injected rollup account migration failure'); END;"
    ))
    .unwrap();
    let before = original_rows(&old);
    let cursor_before = cursor(&old);
    old.close().unwrap();

    // A failed migration leaves schema 55 untouched, without either column.
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(56)
    );
    let oracle = Connection::open(&path).unwrap();
    let columns = |connection: &Connection| -> Vec<String> {
        let mut statement = connection
            .prepare("SELECT name FROM pragma_table_info('consumption_tier5m') ORDER BY cid")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert!(!columns(&oracle).contains(&"credits".to_owned()));
    assert_eq!(original_rows(&oracle), before);
    oracle
        .execute_batch("DROP TRIGGER refuse_account_columns")
        .unwrap();

    let storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 67);
    let columns = columns(&oracle);
    assert_eq!(&columns[columns.len() - 2..], ["credits", "service_tier"]);
    // No row was rewritten: every original cell is the same, both new cells are absent,
    // and the change cursor did not move.
    assert_eq!(original_rows(&oracle), before);
    assert_eq!(cursor(&oracle), cursor_before);
    let absent: i64 = oracle
        .query_row(
            "SELECT count(*) FROM consumption_tier5m WHERE credits IS NULL AND service_tier IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(absent, 2);

    // A change to either new column advances the cursor, an unchanged write does not,
    // and the size limits hold.
    let write = |sql: &str| oracle.execute(sql, []);
    write("UPDATE consumption_tier5m SET credits='{\"balance\":\"1\"}' WHERE from_ms=0").unwrap();
    assert_eq!(cursor(&oracle), cursor_before + 1);
    write("UPDATE consumption_tier5m SET credits='{\"balance\":\"1\"}' WHERE from_ms=0").unwrap();
    assert_eq!(cursor(&oracle), cursor_before + 1);
    write("UPDATE consumption_tier5m SET service_tier='priority' WHERE from_ms=0").unwrap();
    assert_eq!(cursor(&oracle), cursor_before + 2);
    assert!(
        write(&format!(
            "UPDATE consumption_tier5m SET service_tier='{}' WHERE from_ms=0",
            "x".repeat(25)
        ))
        .is_err()
    );
    assert!(
        write(&format!(
            "UPDATE consumption_tier5m SET credits='{}' WHERE from_ms=0",
            "x".repeat(257)
        ))
        .is_err()
    );
    drop(storage);
}
