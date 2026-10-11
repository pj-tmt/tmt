use super::support::{DAY_MS, Fixture, NOW_MS, endpoint, prepare_input, service};
use rusqlite::{Connection, OpenFlags};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{Originator, RequestRoute, ResponseProof, SubmitResponse, correlation},
};

fn sender(f: &mut Fixture) -> String {
    create_or_resolve(&mut f.storage, "Flow sender", Lifetime::Saved)
        .unwrap()
        .identity
        .id
}
fn queue(f: &mut Fixture, sender: &str, id: &str) {
    let mut input = prepare_input(
        f,
        id,
        endpoint("%1", 42),
        false,
        NOW_MS + 3_600_000,
        Originator::Explicit(sender.into()),
        false,
    );
    input.message = "é\0秘".into();
    input.route = RequestRoute::Inbox {
        recipient_identity_id: f.identity_id.clone(),
    };
    service(f)
        .enqueue(input, format!("attempt-{id}"), 1)
        .unwrap();
}
fn reply(f: &mut Fixture, id: &str) {
    let route = RequestRoute::Inbox {
        recipient_identity_id: f.identity_id.clone(),
    };
    service(f)
        .submit_response(SubmitResponse {
            request_id: id.into(),
            proof: ResponseProof::Compact(correlation::response_token(
                id,
                &format!("attempt-{id}"),
                &route,
            )),
            body: "answer é".into(),
        })
        .unwrap();
}
fn totals(f: &Fixture, kind: &str) -> (u64, u64) {
    let c = Connection::open_with_flags(&f.database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    c.query_row(
        "SELECT COALESCE(SUM(count),0), COALESCE(SUM(bytes),0) FROM exchange_flow WHERE kind = ?",
        [kind],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
    )
    .unwrap()
}

#[test]
fn exchange_flow_retains_body_free_utf8_totals_and_first_final_cardinality() {
    let mut f = Fixture::new();
    let origin = sender(&mut f);
    queue(&mut f, &origin, "one");
    reply(&mut f, "one");
    reply(&mut f, "one");
    assert_eq!(totals(&f, "talk"), (1, 6));
    assert_eq!(totals(&f, "reply"), (1, 9));
    service(&mut f).get_response("one").unwrap();
    assert_eq!(totals(&f, "result"), (0, 0));
    let c = Connection::open_with_flags(&f.database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let cols = c
        .prepare("PRAGMA table_info(exchange_flow)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        cols,
        [
            "sender_identity_id",
            "recipient_identity_id",
            "kind",
            "bucket_start_ms",
            "count",
            "bytes"
        ]
    );
    drop(c);
    f.set_now(NOW_MS + 2 * DAY_MS);
    service(&mut f).cleanup().unwrap();
    assert_eq!(totals(&f, "talk"), (1, 6));
    assert_eq!(totals(&f, "reply"), (1, 9));
}

#[test]
fn exchange_flow_ack_counts_only_newly_covered_exchanges_in_both_scopes() {
    let mut f = Fixture::new();
    let origin = sender(&mut f);
    queue(&mut f, &origin, "one");
    queue(&mut f, &origin, "two");
    let revision = service(&mut f)
        .show_exchange(&origin, "one")
        .unwrap()
        .exchange
        .revision;
    service(&mut f)
        .acknowledge_exchange(&origin, "one", revision)
        .unwrap();
    service(&mut f)
        .acknowledge_exchange(&origin, "one", revision)
        .unwrap();
    service(&mut f).acknowledge_all_exchanges(&origin).unwrap();
    service(&mut f).acknowledge_all_exchanges(&origin).unwrap();
    assert_eq!(totals(&f, "ack"), (2, 0));
    let recipient = f.identity_id.clone();
    service(&mut f)
        .acknowledge_all_incoming_requests(&recipient)
        .unwrap();
    service(&mut f)
        .acknowledge_all_incoming_requests(&recipient)
        .unwrap();
    assert_eq!(totals(&f, "ack"), (4, 0));
    reply(&mut f, "one");
    service(&mut f).acknowledge_all_exchanges(&origin).unwrap();
    assert_eq!(totals(&f, "ack"), (5, 0));
}

#[test]
fn exchange_flow_explicit_retrieval_counts_without_polling_or_reader_attribution() {
    let mut f = Fixture::new();
    let origin = sender(&mut f);
    queue(&mut f, &origin, "one");
    reply(&mut f, "one");
    service(&mut f).get_response("one").unwrap();
    service(&mut f).get_response_by_prefix("one").unwrap();
    for _ in 0..2 {
        service(&mut f).retrieve_response_by_prefix("one").unwrap();
    }
    service(&mut f)
        .retrieve_response_by_prefix("unknown")
        .unwrap();
    assert_eq!(totals(&f, "result"), (2, 0));
    let recipient = f.identity_id.clone();
    let report = service(&mut f).exchange_flow(DAY_MS).unwrap();
    let result = report
        .rows
        .iter()
        .find(|r| r.kind.as_str() == "result")
        .unwrap();
    assert_eq!(
        result.sender_identity_id.as_deref(),
        Some(recipient.as_str())
    );
    assert_eq!(
        result.recipient_identity_id.as_deref(),
        Some(origin.as_str())
    );
    service(&mut f).exchange_flow(DAY_MS).unwrap();
    assert_eq!(totals(&f, "result"), (2, 0));
}

#[test]
fn exchange_flow_keeps_whole_buckets_for_thirty_days_independent_of_body_retention() {
    let mut f = Fixture::new();
    let origin = sender(&mut f);
    let bucket = NOW_MS / 1_800_000 * 1_800_000;
    f.set_now(bucket + 1_800_000 - 1);
    queue(&mut f, &origin, "old");
    f.set_now(bucket + 1_800_000);
    queue(&mut f, &origin, "new");
    let rows = service(&mut f).exchange_flow(DAY_MS).unwrap().rows;
    assert_eq!(
        rows.iter().map(|r| r.bucket_start_ms).collect::<Vec<_>>(),
        [bucket, bucket + 1_800_000]
    );
    f.set_now(bucket + 1_800_000 + 30 * DAY_MS - 1);
    service(&mut f).cleanup().unwrap();
    assert_eq!(totals(&f, "talk"), (2, 12));
    f.set_now(bucket + 1_800_000 + 30 * DAY_MS);
    service(&mut f).cleanup().unwrap();
    assert_eq!(totals(&f, "talk"), (1, 6));
    let c = Connection::open_with_flags(&f.database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM request_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(c);
    assert!(service(&mut f).exchange_flow(30 * DAY_MS + 1).is_err());
    assert!(service(&mut f).exchange_flow(0).is_err());
}

#[test]
fn exchange_flow_failure_rolls_back_preparation_and_counter_exhaustion_rolls_back_final() {
    let mut f = Fixture::new();
    let origin = sender(&mut f);
    let c = Connection::open(&f.database).unwrap();
    c.execute_batch("CREATE TRIGGER reject_flow BEFORE INSERT ON exchange_flow BEGIN SELECT RAISE(ABORT,'fixture refusal'); END;").unwrap();
    let mut input = prepare_input(
        &f,
        "refused",
        endpoint("%1", 42),
        false,
        NOW_MS + 3_600_000,
        Originator::Explicit(origin.clone()),
        false,
    );
    input.route = RequestRoute::Inbox {
        recipient_identity_id: f.identity_id.clone(),
    };
    assert!(
        service(&mut f)
            .enqueue(input, "attempt-refused".into(), 1)
            .is_err()
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM request_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER reject_flow").unwrap();
    queue(&mut f, &origin, "one");
    c.execute(
        "INSERT INTO exchange_flow VALUES (?1,?2,'reply',?3,9007199254740991,0)",
        rusqlite::params![
            f.identity_id,
            origin,
            (NOW_MS / 1_800_000 * 1_800_000) as i64
        ],
    )
    .unwrap();
    let route = RequestRoute::Inbox {
        recipient_identity_id: f.identity_id.clone(),
    };
    assert!(
        service(&mut f)
            .submit_response(SubmitResponse {
                request_id: "one".into(),
                proof: ResponseProof::Compact(correlation::response_token(
                    "one",
                    "attempt-one",
                    &route
                )),
                body: "rejected".into()
            })
            .is_err()
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM request_responses", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT response_submitted_at_ms FROM request_attempts WHERE request_id='one'",
            [],
            |r| r.get::<_, Option<i64>>(0)
        )
        .unwrap(),
        None
    );
    assert_eq!(totals(&f, "reply"), (9_007_199_254_740_991, 0));
}

#[test]
fn exchange_flow_keeps_recorded_uuids_unknown_participants_and_an_observation_cursor() {
    let mut f = Fixture::new();
    let origin = sender(&mut f);
    queue(&mut f, &origin, "known");
    let mut input = prepare_input(
        &f,
        "unknown-pair",
        endpoint("%1", 42),
        false,
        NOW_MS + 3_600_000,
        Originator::Unknown,
        false,
    );
    input.recipient_identity_id = None;
    service(&mut f)
        .prepare(input, "unknown-attempt".into(), 1)
        .unwrap();
    Connection::open(&f.database)
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms = ? WHERE id = ?",
            rusqlite::params![NOW_MS as i64, origin],
        )
        .unwrap();
    let replacement = sender(&mut f);
    assert_ne!(replacement, origin);
    // A normal request read at this time would housekeep expired bodies. Flow must not.
    f.set_now(NOW_MS + 2 * DAY_MS);
    let cursor = f.storage.change_cursor().unwrap();
    let report = service(&mut f).exchange_flow(3 * DAY_MS).unwrap();
    assert_eq!(f.storage.change_cursor().unwrap(), cursor);
    assert_eq!(report.rows.len(), 2);
    let known = report
        .rows
        .iter()
        .find(|r| r.sender_identity_id.as_deref() == Some(origin.as_str()))
        .unwrap();
    assert_eq!(
        known.recipient_identity_id.as_deref(),
        Some(f.identity_id.as_str())
    );
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.sender_identity_id.is_none() && r.recipient_identity_id.is_none())
    );
    assert!(
        !report
            .rows
            .iter()
            .any(|r| r.sender_identity_id.as_deref() == Some(replacement.as_str()))
    );
    let c = Connection::open_with_flags(&f.database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM request_attempts WHERE message_text IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
