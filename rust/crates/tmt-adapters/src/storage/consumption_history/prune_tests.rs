//! Expired usage rows are removed when a sample enters a new rollup bucket (#5019),
//! not on every 5-second tick.
use super::{
    tests::{fixture, latest, source},
    *,
};

fn fine_rows(storage: &Storage, id: &str) -> i64 {
    storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM consumption_buckets WHERE identity_id=?",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

fn rollup_rows(storage: &Storage, id: &str) -> i64 {
    storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM consumption_tier5m WHERE identity_id=?",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn expired_rows_wait_for_the_sample_that_enters_the_next_rollup_bucket() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let first = ROLLUP_BUCKET_MS * 2;
    let mut now = first;
    let mut sequence = 0;
    while now < first + HISTORY_MS + ROLLUP_BUCKET_MS {
        sequence += 1;
        record_sample(
            storage.connection().unwrap(),
            &id,
            Some(latest(sequence, sequence, sequence, now)),
            now,
        )
        .unwrap();
        now += BUCKET_MS;
    }
    // Everything since the first sample is still stored: the only prune that ran so far
    // was at the two-hour mark, when nothing was older than the window yet.
    assert_eq!(
        fine_rows(&storage, &id),
        ((HISTORY_MS + ROLLUP_BUCKET_MS) / BUCKET_MS) as i64
    );
    assert_eq!(now % ROLLUP_BUCKET_MS, 0);
    // The sample that enters the next rollup bucket removes everything older than the window.
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(latest(sequence + 1, sequence + 1, sequence + 1, now)),
        now,
    )
    .unwrap();
    assert_eq!(
        fine_rows(&storage, &id),
        (HISTORY_MS / BUCKET_MS) as i64 + 1
    );
    storage.close().unwrap();
}

#[test]
fn a_retired_identitys_rows_expire_with_the_next_sample_of_any_identity() {
    // The prune also takes a bounded few of anyone's expired rows, so an identity
    // that never writes again does not keep them forever.
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let other = "other-identity";
    storage
        .connection()
        .unwrap()
        .execute_batch(&format!(
            "PRAGMA foreign_keys = OFF;
             INSERT INTO consumption_buckets(identity_id,from_ms,input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous) \
             VALUES('{other}',0,0,0,0,0,1,0,0),('{other}',5000,0,0,0,0,1,0,0);
             INSERT INTO consumption_tier5m(identity_id,from_ms,input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous) \
             VALUES('{other}',0,0,0,0,0,1,0,0);
             PRAGMA foreign_keys = ON;"
        ))
        .unwrap();
    let now = ROLLUP_MS + ROLLUP_BUCKET_MS * 3;
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(latest(1, 1, 1, now)),
        now,
    )
    .unwrap();
    let count = |table: &str| -> i64 {
        storage
            .connection()
            .unwrap()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE identity_id=?"),
                [other],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(count("consumption_buckets"), 0);
    assert_eq!(count("consumption_tier5m"), 0);
    assert!(rollup_rows(&storage, &id) >= 1);
    storage.close().unwrap();
}
