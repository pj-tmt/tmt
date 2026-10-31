//! SQLite snapshots keep all bytes except the destination's cache/change header fields.
use rusqlite::{Connection, OpenFlags};
use std::{fs, ops::Range, path::Path};

// Database header fields: https://www.sqlite.org/fileformat.html#the_database_header
const FILE_CHANGE_COUNTER: Range<usize> = 24..28;
const SCHEMA_COOKIE: Range<usize> = 40..44;
const VERSION_VALID_FOR: Range<usize> = 92..96;
const USER_VERSION: Range<usize> = 60..64;

pub fn assert_snapshot(path: &Path, before: &[u8]) -> Connection {
    let snapshot = fs::read(path).unwrap();
    assert_eq!(snapshot.len(), before.len());
    for bytes in [before, &snapshot] {
        assert_eq!(bytes[FILE_CHANGE_COUNTER], bytes[VERSION_VALID_FOR]);
    }
    let mut comparable = snapshot.clone();
    for field in [FILE_CHANGE_COUNTER, SCHEMA_COOKIE, VERSION_VALID_FOR] {
        comparable[field.clone()].copy_from_slice(&before[field]);
    }
    // Includes all row data, schema, free pages and the SQLite version field at offset 96.
    assert_eq!(comparable, before);
    // NOFOLLOW also rejects symlinked parents, including macOS /tmp -> /private/tmp.
    let saved = Connection::open_with_flags(
        path.canonicalize().unwrap(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    assert_eq!(
        saved
            .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        saved
            .pragma_query_value(None, "schema_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        u32::from_be_bytes(snapshot[SCHEMA_COOKIE].try_into().unwrap())
    );
    assert_eq!(
        saved
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        u32::from_be_bytes(before[USER_VERSION].try_into().unwrap())
    );
    saved
}
