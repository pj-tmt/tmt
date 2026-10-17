use super::support::{DAY_MS, Fixture, NOW_MS, endpoint, service};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use tmt_core::identity::{Lifetime, create_or_resolve};
use tmt_core::request::attention::{AttentionRejection, Exchange, FinalState};
use tmt_core::request::{
    AttemptStatus, Originator, PrepareRequest, RequestError, RequestPrompt, RequestRoute,
    ResponseProof, Settlement, SubmitResponse,
};

fn prepare_sent(
    fixture: &mut Fixture,
    identity_id: &str,
    request_id: &str,
    pane_id: &str,
    pane_pid: u64,
    message: &str,
    retention_days: u64,
) -> (String, tmt_core::request::RequestEndpoint) {
    let target = endpoint(pane_id, pane_pid);
    let prepared = service(fixture)
        .prepare(
            PrepareRequest {
                origin: tmt_core::request::RequestOrigin::Person,
                room_id: None,
                kind: tmt_core::request::RequestKind::Request,
                request_id: request_id.into(),
                message: message.into(),
                route: RequestRoute::Pane(target.clone()),
                wait: false,
                expires_at_ms: NOW_MS + 3_600_001,
                originator: Originator::Explicit(identity_id.into()),
                recipient_identity_id: None,
                preamble: None,
            },
            format!("attempt-{request_id}"),
            retention_days,
        )
        .expect("prepare attention request");
    service(fixture)
        .begin_send(&prepared.attempt_id)
        .expect("begin attention request");
    service(fixture)
        .settle(&prepared.attempt_id, Settlement::Sent)
        .expect("settle attention request");
    (prepared.attempt_id, target)
}

fn submit_final(
    fixture: &mut Fixture,
    request_id: &str,
    attempt_id: &str,
    target: tmt_core::request::RequestEndpoint,
    body: &str,
) -> tmt_core::request::FinalResponse {
    service(fixture)
        .submit_response(SubmitResponse {
            request_id: request_id.into(),
            proof: ResponseProof::Recorded {
                attempt_id: attempt_id.into(),
                endpoint: target,
            },
            body: body.into(),
        })
        .expect("submit attention final")
}

fn create_identity(fixture: &mut Fixture, name: &str) -> String {
    create_or_resolve(&mut fixture.storage, name, Lifetime::Saved)
        .expect("create attention identity")
        .identity
        .id
}

#[derive(Debug, PartialEq, Eq)]
struct AttentionSql {
    counter: Option<(i64, i64)>,
    attempts: Vec<(String, i64, i64)>,
}

#[derive(Debug, PartialEq, Eq)]
struct RequestMetadataSql {
    prompt: Option<String>,
    prompt_bytes: Option<i64>,
    prompt_expires_at_ms: Option<i64>,
    retention_expires_at_ms: i64,
    response_body: Option<String>,
    response_body_bytes: Option<i64>,
    response_submitted_at_ms: Option<i64>,
    response_expires_at_ms: Option<i64>,
}

fn attention_sql(fixture: &Fixture, identity_id: &str) -> AttentionSql {
    let connection =
        Connection::open_with_flags(&fixture.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open read-only attention oracle");
    let counter = connection
        .query_row(
            "SELECT latest_revision, acknowledged_through
             FROM request_attention_identities WHERE identity_id = ?",
            [identity_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .expect("read attention counter");
    let mut statement = connection
        .prepare(
            "SELECT request_id, attention_revision, attention_acknowledged_revision
             FROM request_attempts WHERE originator_identity_id = ?
             ORDER BY attention_revision, request_id",
        )
        .expect("prepare attention rows");
    let attempts = statement
        .query_map([identity_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("query attention rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("decode attention rows");
    AttentionSql { counter, attempts }
}

fn request_ids(items: &[Exchange]) -> Vec<&str> {
    items.iter().map(|item| item.request_id.as_str()).collect()
}

fn request_metadata_sql(fixture: &Fixture, request_id: &str) -> RequestMetadataSql {
    let connection =
        Connection::open_with_flags(&fixture.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open read-only request metadata oracle");
    connection
        .query_row(
            "SELECT a.message_text, a.message_bytes, a.message_expires_at_ms,
                    a.retention_expires_at_ms, r.body, r.body_bytes,
                    r.submitted_at_ms, r.response_expires_at_ms
             FROM request_attempts a
             LEFT JOIN request_responses r ON r.request_id = a.request_id
             WHERE a.request_id = ?",
            [request_id],
            |row| {
                Ok(RequestMetadataSql {
                    prompt: row.get(0)?,
                    prompt_bytes: row.get(1)?,
                    prompt_expires_at_ms: row.get(2)?,
                    retention_expires_at_ms: row.get(3)?,
                    response_body: row.get(4)?,
                    response_body_bytes: row.get(5)?,
                    response_submitted_at_ms: row.get(6)?,
                    response_expires_at_ms: row.get(7)?,
                })
            },
        )
        .expect("read request metadata")
}

#[test]
fn lists_one_identity_in_revision_order_with_bounded_pagination() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    for (request_id, pane_id, pane_pid) in [
        ("attention-page-1", "%10", 110),
        ("attention-page-2", "%11", 111),
        ("attention-page-3", "%12", 112),
    ] {
        prepare_sent(
            &mut fixture,
            &owner,
            request_id,
            pane_id,
            pane_pid,
            request_id,
            7,
        );
    }

    let first = service(&mut fixture)
        .list_exchanges(&owner, Some(2), None)
        .expect("list first attention page");
    assert_eq!(
        request_ids(&first.items),
        ["attention-page-1", "attention-page-2"]
    );
    assert_eq!(first.next_after, Some(2));
    assert!(first.items.iter().all(|item| {
        item.delivery == AttemptStatus::Sent
            && item.final_state == FinalState::NotSubmitted
            && !item.acknowledged
            && !item.settled
    }));

    let second = service(&mut fixture)
        .list_exchanges(&owner, Some(2), first.next_after)
        .expect("list second attention page");
    assert_eq!(request_ids(&second.items), ["attention-page-3"]);
    assert_eq!(second.next_after, None);
    let before_invalid = attention_sql(&fixture, &owner);
    for invalid_limit in [0, 201] {
        assert!(matches!(
            service(&mut fixture).list_exchanges(&owner, Some(invalid_limit), None),
            Err(RequestError::Attention(AttentionRejection::Invalid(_)))
        ));
    }
    assert!(matches!(
        service(&mut fixture).list_exchanges(&owner, None, Some(9_007_199_254_740_992)),
        Err(RequestError::Attention(AttentionRejection::Invalid(_)))
    ));
    for invalid_revision in [0, 9_007_199_254_740_992] {
        assert!(matches!(
            service(&mut fixture).acknowledge_exchange(
                &owner,
                "attention-page-1",
                invalid_revision
            ),
            Err(RequestError::Attention(AttentionRejection::Invalid(_)))
        ));
    }
    assert_eq!(attention_sql(&fixture, &owner), before_invalid);
    assert_eq!(
        attention_sql(&fixture, &owner),
        AttentionSql {
            counter: Some((3, 0)),
            attempts: vec![
                ("attention-page-1".into(), 1, 0),
                ("attention-page-2".into(), 2, 0),
                ("attention-page-3".into(), 3, 0),
            ],
        }
    );
}

#[test]
fn metadata_expiry_hides_show_and_ack_at_deadline_equality() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    prepare_sent(
        &mut fixture,
        &owner,
        "attention-expiry",
        "%19",
        119,
        "prompt",
        7,
    );
    let detail = service(&mut fixture)
        .show_exchange(&owner, "attention-expiry")
        .unwrap();
    fixture.set_now(detail.exchange.retention_expires_at_ms);
    assert!(matches!(
        service(&mut fixture).show_exchange(&owner, "attention-expiry"),
        Err(RequestError::Attention(AttentionRejection::NotFound))
    ));
    assert!(matches!(
        service(&mut fixture).acknowledge_exchange(&owner, "attention-expiry", 1),
        Err(RequestError::Attention(AttentionRejection::NotFound))
    ));
    assert!(
        service(&mut fixture)
            .list_exchanges(&owner, None, None)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(attention_sql(&fixture, &owner).counter, Some((1, 0)));
}

#[test]
fn foreign_identity_cannot_list_show_or_ack_another_originator() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    let foreign = create_identity(&mut fixture, "Foreign Attention Owner");
    let (attempt_id, target) = prepare_sent(
        &mut fixture,
        &foreign,
        "attention-foreign",
        "%20",
        120,
        "foreign prompt",
        7,
    );
    let before = attention_sql(&fixture, &foreign);

    let own = service(&mut fixture)
        .list_exchanges(&owner, None, None)
        .expect("list own attention");
    assert!(own.items.is_empty());
    assert!(matches!(
        service(&mut fixture).show_exchange(&owner, "attention-foreign"),
        Err(RequestError::Attention(AttentionRejection::NotFound))
    ));
    assert!(matches!(
        service(&mut fixture).acknowledge_exchange(&owner, "attention-foreign", 1,),
        Err(RequestError::Attention(AttentionRejection::NotFound))
    ));
    let foreign_page = service(&mut fixture)
        .list_exchanges(&foreign, None, None)
        .expect("list foreign attention");
    assert_eq!(request_ids(&foreign_page.items), ["attention-foreign"]);
    assert_eq!(attention_sql(&fixture, &foreign), before);

    submit_final(
        &mut fixture,
        "attention-foreign",
        &attempt_id,
        target,
        "foreign final",
    );
    let detail = service(&mut fixture)
        .show_exchange(&foreign, "attention-foreign")
        .expect("show foreign detail");
    assert!(matches!(
        detail.exchange.final_state,
        FinalState::Retained { .. }
    ));
}

#[test]
fn acknowledged_pending_request_reappears_when_a_late_final_advances_revision() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    let (attempt_id, target) = prepare_sent(
        &mut fixture,
        &owner,
        "attention-late-final",
        "%30",
        130,
        "pending prompt",
        7,
    );
    let pending = service(&mut fixture)
        .list_exchanges(&owner, None, None)
        .expect("list pending attention");
    assert_eq!(pending.items[0].revision, 1);
    assert!(matches!(
        pending.items[0].final_state,
        FinalState::NotSubmitted
    ));
    let retention_expires_at_ms = pending.items[0].retention_expires_at_ms;
    let acknowledged = service(&mut fixture)
        .acknowledge_exchange(&owner, "attention-late-final", 1)
        .expect("acknowledge pending request");
    assert!(acknowledged.changed);
    let acknowledged_detail = service(&mut fixture)
        .show_exchange(&owner, "attention-late-final")
        .expect("show acknowledged pending request");
    assert!(acknowledged_detail.exchange.acknowledged);
    assert!(!acknowledged_detail.exchange.settled);
    assert!(
        service(&mut fixture)
            .list_exchanges(&owner, None, None)
            .expect("list acknowledged pending request")
            .items
            .is_empty()
    );

    submit_final(
        &mut fixture,
        "attention-late-final",
        &attempt_id,
        target,
        "late final",
    );
    let late = service(&mut fixture)
        .list_exchanges(&owner, None, None)
        .expect("list late final");
    assert_eq!(request_ids(&late.items), ["attention-late-final"]);
    assert_eq!(late.items[0].revision, 2);
    assert_eq!(
        late.items[0].retention_expires_at_ms,
        retention_expires_at_ms
    );
    assert!(!late.items[0].acknowledged);
    assert!(!late.items[0].settled);
    assert!(matches!(
        late.items[0].final_state,
        FinalState::Retained { .. }
    ));
    assert_eq!(
        attention_sql(&fixture, &owner),
        AttentionSql {
            counter: Some((2, 0)),
            attempts: vec![("attention-late-final".into(), 2, 1)],
        }
    );
}

#[test]
fn show_preserves_exact_prompt_and_final_body_detail() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    let prompt = "\u{feff} prompt\r\n\u{0000} \u{65e5}\u{672c}\u{8a9e}  ";
    let body = "\u{feff} final\r\n\u{0000}🙂  ";
    let (attempt_id, target) = prepare_sent(
        &mut fixture,
        &owner,
        "attention-exact-detail",
        "%40",
        140,
        prompt,
        7,
    );
    let final_response = submit_final(
        &mut fixture,
        "attention-exact-detail",
        &attempt_id,
        target,
        body,
    );
    let detail = service(&mut fixture)
        .show_exchange(&owner, "attention-exact-detail")
        .expect("show exact attention detail");
    assert!(matches!(
        detail.prompt,
        RequestPrompt::Retained(ref stored)
            if stored.message == prompt && stored.message_bytes == prompt.len() as u64
    ));
    assert!(matches!(
        detail.exchange.final_state,
        FinalState::Retained {
            ref content,
            submitted_at_ms,
            body_bytes,
            expires_at_ms,
        } if content == body
            && submitted_at_ms == final_response.submitted_at_ms
            && body_bytes == body.len() as u64
            && expires_at_ms == final_response.response_expires_at_ms
    ));
    let connection =
        Connection::open_with_flags(&fixture.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open exact detail oracle");
    let stored: (String, i64, String, i64) = connection
        .query_row(
            "SELECT a.message_text, a.message_bytes, r.body, r.body_bytes
             FROM request_attempts a JOIN request_responses r ON r.request_id = a.request_id
             WHERE a.request_id = 'attention-exact-detail'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read exact prompt and final");
    assert_eq!(
        stored,
        (
            prompt.into(),
            prompt.len() as i64,
            body.into(),
            body.len() as i64
        )
    );
}

#[test]
fn list_projects_final_metadata_without_decoding_a_corrupt_body() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    let (attempt_id, target) = prepare_sent(
        &mut fixture,
        &owner,
        "attention-corrupt-body",
        "%45",
        145,
        "metadata prompt",
        7,
    );
    submit_final(
        &mut fixture,
        "attention-corrupt-body",
        &attempt_id,
        target,
        "valid body",
    );
    let writer = Connection::open(&fixture.database).expect("open corrupt-body writer");
    writer
        .execute(
            "UPDATE request_responses
             SET body = CAST(X'FF' AS BLOB), body_bytes = 1
             WHERE request_id = ?",
            params!["attention-corrupt-body"],
        )
        .expect("corrupt only final body bytes");
    drop(writer);

    let page = service(&mut fixture)
        .list_exchanges(&owner, None, None)
        .expect("list metadata without loading final body");
    assert_eq!(request_ids(&page.items), ["attention-corrupt-body"]);
    assert!(matches!(
        page.items[0].final_state,
        FinalState::Retained { body_bytes: 1, .. }
    ));
    assert!(matches!(
        service(&mut fixture).show_exchange(&owner, "attention-corrupt-body"),
        Err(RequestError::Repository(_))
    ));
}

#[test]
fn stale_ack_conflicts_and_repeated_current_ack_is_idempotent() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    let (attempt_id, target) = prepare_sent(
        &mut fixture,
        &owner,
        "attention-stale-ack",
        "%50",
        150,
        "stale ack prompt",
        7,
    );
    let first = service(&mut fixture)
        .acknowledge_exchange(&owner, "attention-stale-ack", 1)
        .expect("acknowledge first revision");
    assert!(first.changed);
    submit_final(
        &mut fixture,
        "attention-stale-ack",
        &attempt_id,
        target,
        "stale ack final",
    );
    let stale = service(&mut fixture).acknowledge_exchange(&owner, "attention-stale-ack", 1);
    assert!(matches!(
        stale,
        Err(RequestError::Attention(
            AttentionRejection::RevisionConflict {
                current: 2,
                expected: 1
            }
        ))
    ));
    assert_eq!(
        attention_sql(&fixture, &owner),
        AttentionSql {
            counter: Some((2, 0)),
            attempts: vec![("attention-stale-ack".into(), 2, 1)],
        }
    );

    fixture.set_now(NOW_MS + DAY_MS);
    let before_current_ack = request_metadata_sql(&fixture, "attention-stale-ack");
    let current = service(&mut fixture)
        .acknowledge_exchange(&owner, "attention-stale-ack", 2)
        .expect("acknowledge current revision");
    assert!(current.changed);
    let repeated = service(&mut fixture)
        .acknowledge_exchange(&owner, "attention-stale-ack", 2)
        .expect("repeat current acknowledgment");
    assert!(!repeated.changed);
    assert_eq!(attention_sql(&fixture, &owner).attempts[0].2, 2);
    assert_eq!(
        request_metadata_sql(&fixture, "attention-stale-ack"),
        before_current_ack
    );
    let settled_detail = service(&mut fixture)
        .show_exchange(&owner, "attention-stale-ack")
        .expect("show acknowledged final");
    assert!(settled_detail.exchange.acknowledged);
    assert!(settled_detail.exchange.settled);
}

#[test]
fn ackall_cutoff_leaves_late_final_and_new_request_unacknowledged() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    let (first_attempt, first_target) = prepare_sent(
        &mut fixture,
        &owner,
        "attention-ackall-first",
        "%60",
        160,
        "first prompt",
        7,
    );
    let cutoff = service(&mut fixture)
        .acknowledge_all_exchanges(&owner)
        .expect("ackall first cutoff");
    assert_eq!(cutoff, 1);
    assert!(
        service(&mut fixture)
            .list_exchanges(&owner, None, None)
            .expect("list after first ackall")
            .items
            .is_empty()
    );

    let (_second_attempt, _second_target) = prepare_sent(
        &mut fixture,
        &owner,
        "attention-ackall-second",
        "%61",
        161,
        "second prompt",
        7,
    );
    let second_page = service(&mut fixture)
        .list_exchanges(&owner, None, None)
        .expect("list request after cutoff");
    assert_eq!(request_ids(&second_page.items), ["attention-ackall-second"]);
    assert_eq!(second_page.items[0].revision, 2);

    submit_final(
        &mut fixture,
        "attention-ackall-first",
        &first_attempt,
        first_target,
        "late first final",
    );
    let late_page = service(&mut fixture)
        .list_exchanges(&owner, None, None)
        .expect("list late first final");
    assert_eq!(
        request_ids(&late_page.items),
        ["attention-ackall-second", "attention-ackall-first"]
    );
    assert_eq!(late_page.items[0].revision, 2);
    assert_eq!(late_page.items[1].revision, 3);

    let second_cutoff = service(&mut fixture)
        .acknowledge_all_exchanges(&owner)
        .expect("ackall late cutoff");
    assert_eq!(second_cutoff, 3);
    let (_third_attempt, _third_target) = prepare_sent(
        &mut fixture,
        &owner,
        "attention-ackall-third",
        "%62",
        162,
        "third prompt",
        7,
    );
    let new_page = service(&mut fixture)
        .list_exchanges(&owner, None, None)
        .expect("list request after second cutoff");
    assert_eq!(request_ids(&new_page.items), ["attention-ackall-third"]);
    assert_eq!(new_page.items[0].revision, 4);
    assert!(!new_page.items[0].acknowledged);
    assert_eq!(attention_sql(&fixture, &owner).counter, Some((4, 3)));
}

#[test]
fn final_state_distinguishes_expired_body_from_unavailable_body_at_retention_boundary() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    let (expired_attempt, expired_target) = prepare_sent(
        &mut fixture,
        &owner,
        "attention-expired-final",
        "%70",
        170,
        "expired prompt",
        1,
    );
    fixture.set_now(NOW_MS + 2 * DAY_MS);
    let expired_response = submit_final(
        &mut fixture,
        "attention-expired-final",
        &expired_attempt,
        expired_target,
        "expired final",
    );
    fixture.set_now(expired_response.response_expires_at_ms);
    let expired = service(&mut fixture)
        .show_exchange(&owner, "attention-expired-final")
        .expect("show expired final");
    assert!(matches!(
        expired.exchange.final_state,
        FinalState::Expired {
            submitted_at_ms,
            expires_at_ms,
        } if submitted_at_ms == expired_response.submitted_at_ms
            && expires_at_ms == expired_response.response_expires_at_ms
    ));

    let mut unavailable_fixture = Fixture::new();
    let unavailable_owner = unavailable_fixture.identity_id.clone();
    let (unavailable_attempt, unavailable_target) = prepare_sent(
        &mut unavailable_fixture,
        &unavailable_owner,
        "attention-unavailable-final",
        "%71",
        171,
        "unavailable prompt",
        7,
    );
    submit_final(
        &mut unavailable_fixture,
        "attention-unavailable-final",
        &unavailable_attempt,
        unavailable_target,
        "unavailable final",
    );
    let writer = Connection::open(&unavailable_fixture.database).expect("open fixture writer");
    writer
        .execute(
            "DELETE FROM request_responses WHERE request_id = ?",
            params!["attention-unavailable-final"],
        )
        .expect("remove only unavailable body");
    drop(writer);
    let unavailable = service(&mut unavailable_fixture)
        .show_exchange(&unavailable_owner, "attention-unavailable-final")
        .expect("show unavailable final");
    assert!(matches!(
        unavailable.exchange.final_state,
        FinalState::Unavailable {
            submitted_at_ms,
            expires_at_ms,
        } if submitted_at_ms == NOW_MS && expires_at_ms == NOW_MS + 7 * DAY_MS
    ));
    let oracle = Connection::open_with_flags(
        &unavailable_fixture.database,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open final-state oracle");
    let marker: Option<i64> = oracle
        .query_row(
            "SELECT response_submitted_at_ms FROM request_attempts WHERE request_id = ?",
            ["attention-unavailable-final"],
            |row| row.get(0),
        )
        .expect("read unavailable marker");
    assert_eq!(marker, Some(NOW_MS as i64));
}

#[test]
fn ackall_trigger_failure_rolls_back_watermark_and_acknowledgments() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    let (_attempt_id, _target) = prepare_sent(
        &mut fixture,
        &owner,
        "attention-rollback",
        "%80",
        180,
        "rollback prompt",
        1,
    );
    let before = attention_sql(&fixture, &owner);
    let metadata_before = request_metadata_sql(&fixture, "attention-rollback");
    fixture.set_now(NOW_MS + DAY_MS);
    let writer = Connection::open(&fixture.database).expect("open rollback writer");
    writer
        .execute_batch(
            "CREATE TRIGGER reject_attention_ackall
             BEFORE UPDATE OF acknowledged_through ON request_attention_identities
             BEGIN SELECT RAISE(ABORT, 'injected attention failure'); END;",
        )
        .expect("install attention rollback trigger");
    assert!(matches!(
        service(&mut fixture).acknowledge_all_exchanges(&owner),
        Err(RequestError::Repository(_))
    ));
    assert_eq!(attention_sql(&fixture, &owner), before);
    assert_eq!(
        request_metadata_sql(&fixture, "attention-rollback"),
        metadata_before
    );
    writer
        .execute_batch("DROP TRIGGER reject_attention_ackall")
        .expect("remove attention rollback trigger");
    let acknowledged = service(&mut fixture)
        .acknowledge_all_exchanges(&owner)
        .expect("ackall after rollback");
    assert_eq!(acknowledged, 1);
    assert_eq!(attention_sql(&fixture, &owner).counter, Some((1, 1)));
    let metadata_after = request_metadata_sql(&fixture, "attention-rollback");
    assert_eq!(metadata_after.prompt, None);
    assert_eq!(metadata_after.prompt_bytes, None);
    assert_eq!(
        metadata_after.prompt_expires_at_ms,
        metadata_before.prompt_expires_at_ms
    );
    assert_eq!(
        metadata_after.retention_expires_at_ms,
        metadata_before.retention_expires_at_ms
    );
}

#[test]
fn inbox_route_keeps_recipient_request_attention_independent_from_originator_response_attention() {
    let mut fixture = Fixture::new();
    let sender = fixture.identity_id.clone();
    let receiver = create_or_resolve(&mut fixture.storage, "Inbox Receiver", Lifetime::Saved)
        .expect("create inbox receiver")
        .identity;
    let route = RequestRoute::Inbox {
        recipient_identity_id: receiver.id.clone(),
    };
    let prepared = service(&mut fixture)
        .prepare(
            PrepareRequest {
                origin: tmt_core::request::RequestOrigin::Person,
                room_id: None,
                kind: tmt_core::request::RequestKind::Request,
                request_id: "inbox-roundtrip".into(),
                message: "inspect this".into(),
                route: route.clone(),
                wait: false,
                expires_at_ms: NOW_MS + 3_600_000,
                originator: Originator::Explicit(sender.clone()),
                recipient_identity_id: Some(receiver.id.clone()),
                preamble: None,
            },
            "inbox-attempt".into(),
            7,
        )
        .expect("prepare inbox request");
    assert_eq!(
        service(&mut fixture)
            .incoming_watermark(&receiver.id, None)
            .unwrap(),
        0
    );
    assert!(
        service(&mut fixture)
            .list_incoming(&receiver.id, None, None, None)
            .unwrap()
            .items
            .is_empty()
    );
    service(&mut fixture)
        .queue(&prepared.attempt_id)
        .expect("queue inbox request");

    let incoming = service(&mut fixture)
        .list_incoming(&receiver.id, None, None, None)
        .expect("list recipient inbox");
    assert_eq!(incoming.items.len(), 1);
    assert_eq!(
        incoming.items[0].kind,
        tmt_core::request::attention::IncomingKind::Request
    );
    assert_eq!(incoming.items[0].exchange.revision, 1);
    assert_eq!(
        incoming.items[0].sender_identity_id.as_deref(),
        Some(sender.as_str())
    );
    service(&mut fixture)
        .acknowledge_incoming_request(&receiver.id, "inbox-roundtrip", 1)
        .expect("ack recipient role");
    assert!(
        service(&mut fixture)
            .list_incoming(&receiver.id, None, None, None)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        service(&mut fixture)
            .list_exchanges(&sender, None, None)
            .unwrap()
            .items
            .len(),
        1
    );

    fixture.set_now(NOW_MS + 1);
    let response = service(&mut fixture)
        .submit_response(SubmitResponse {
            request_id: "inbox-roundtrip".into(),
            proof: ResponseProof::Compact(tmt_core::request::correlation::response_token(
                "inbox-roundtrip",
                "inbox-attempt",
                &route,
            )),
            body: "done".into(),
        })
        .expect("submit inbox response");
    assert_eq!(response.body, "done");
    let sender_incoming = service(&mut fixture)
        .list_incoming(&sender, None, None, None)
        .expect("list originator results");
    assert_eq!(sender_incoming.items.len(), 1);
    assert_eq!(
        sender_incoming.items[0].kind,
        tmt_core::request::attention::IncomingKind::Response
    );
    assert_eq!(sender_incoming.items[0].exchange.revision, 2);
    assert!(
        service(&mut fixture)
            .list_incoming(&receiver.id, None, None, None)
            .unwrap()
            .items
            .is_empty()
    );
}

fn queue_inbox(
    fixture: &mut Fixture,
    request_id: &str,
    sender: &str,
    recipient: &str,
) -> RequestRoute {
    let route = RequestRoute::Inbox {
        recipient_identity_id: recipient.into(),
    };
    let prepared = service(fixture)
        .prepare(
            PrepareRequest {
                origin: tmt_core::request::RequestOrigin::Person,
                room_id: None,
                kind: tmt_core::request::RequestKind::Request,
                request_id: request_id.into(),
                message: format!("prompt for {request_id}"),
                route: route.clone(),
                wait: false,
                expires_at_ms: NOW_MS + 3_600_000,
                originator: Originator::Explicit(sender.into()),
                recipient_identity_id: Some(recipient.into()),
                preamble: None,
            },
            format!("{request_id}-attempt"),
            7,
        )
        .expect("prepare inbox request");
    service(fixture)
        .queue(&prepared.attempt_id)
        .expect("queue inbox request");
    route
}

#[test]
fn incoming_response_pagination_skips_earlier_non_response_attention_rows() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    for index in 1..=3 {
        prepare_sent(
            &mut fixture,
            &owner,
            &format!("pending-before-response-{index}"),
            &format!("%7{index}"),
            700 + index,
            "pending",
            7,
        );
    }
    for index in 1..=3 {
        let request_id = format!("incoming-response-{index}");
        let (attempt_id, target) = prepare_sent(
            &mut fixture,
            &owner,
            &request_id,
            &format!("%8{index}"),
            800 + index,
            "respond later",
            7,
        );
        submit_final(&mut fixture, &request_id, &attempt_id, target, "done");
    }

    let first = service(&mut fixture)
        .list_incoming(&owner, None, Some(2), None)
        .expect("list first incoming response page");
    assert_eq!(first.items.len(), 2);
    assert!(
        first
            .items
            .iter()
            .all(|item| { item.kind == tmt_core::request::attention::IncomingKind::Response })
    );
    assert_eq!(first.items[0].exchange.request_id, "incoming-response-1");
    assert_eq!(first.items[1].exchange.request_id, "incoming-response-2");
    let cursor = first
        .next_after
        .expect("more response attention remains after first page");
    assert_eq!(cursor, first.items[1].exchange.revision);

    let second = service(&mut fixture)
        .list_incoming(&owner, None, Some(2), Some(cursor))
        .expect("list second incoming response page");
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].exchange.request_id, "incoming-response-3");
    assert_eq!(second.next_after, None);
}

#[test]
fn queue_rejects_a_recipient_retired_after_resolution_without_retargeting_reused_name() {
    let mut fixture = Fixture::new();
    let sender = fixture.identity_id.clone();
    let original = create_or_resolve(&mut fixture.storage, "Queue Race", Lifetime::Saved)
        .unwrap()
        .identity;
    let route = RequestRoute::Inbox {
        recipient_identity_id: original.id.clone(),
    };
    let prepared = service(&mut fixture)
        .prepare(
            PrepareRequest {
                origin: tmt_core::request::RequestOrigin::Person,
                room_id: None,
                kind: tmt_core::request::RequestKind::Request,
                request_id: "retired-before-queue".into(),
                message: "must not move to replacement".into(),
                route,
                wait: true,
                expires_at_ms: NOW_MS + 3_600_000,
                originator: Originator::Explicit(sender),
                recipient_identity_id: Some(original.id.clone()),
                preamble: None,
            },
            "retired-before-queue-attempt".into(),
            7,
        )
        .unwrap();
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms = ? WHERE id = ?",
            params![NOW_MS as i64, original.id],
        )
        .unwrap();
    let replacement = create_or_resolve(&mut fixture.storage, "Queue Race", Lifetime::Saved)
        .unwrap()
        .identity;
    assert_ne!(replacement.id, original.id);

    assert!(matches!(
        service(&mut fixture).queue(&prepared.attempt_id),
        Err(RequestError::NotFound)
    ));
    let connection = Connection::open(&fixture.database).unwrap();
    let state: (String, i64, i64, Option<i64>, Option<i64>) = connection
        .query_row(
            "SELECT status, recipient_attention_revision, wait_active,
                    wait_released_at_ms, settled_at_ms
             FROM request_attempts WHERE request_id = 'retired-before-queue'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(state.0, "definitely_failed");
    assert_eq!(state.1, 0);
    assert_eq!(state.2, 0);
    assert_eq!(state.3, Some(NOW_MS as i64));
    assert_eq!(state.4, Some(NOW_MS as i64));
    let counters: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM request_recipient_attention_identities
             WHERE identity_id IN (?, ?)",
            params![original.id, replacement.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(counters, 0);
}

#[test]
fn recipient_ackall_is_snapshot_bounded_and_exact_ack_rejects_a_stale_revision() {
    let mut fixture = Fixture::new();
    let sender = fixture.identity_id.clone();
    let receiver = create_or_resolve(&mut fixture.storage, "Ackall Receiver", Lifetime::Saved)
        .unwrap()
        .identity;
    queue_inbox(&mut fixture, "inbox-before-ackall", &sender, &receiver.id);
    assert_eq!(
        service(&mut fixture)
            .acknowledge_all_incoming_requests(&receiver.id)
            .unwrap(),
        1
    );

    queue_inbox(&mut fixture, "inbox-after-ackall", &sender, &receiver.id);
    let page = service(&mut fixture)
        .list_incoming(&receiver.id, None, None, None)
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].exchange.request_id, "inbox-after-ackall");
    assert_eq!(page.items[0].exchange.revision, 2);
    assert!(matches!(
        service(&mut fixture).acknowledge_incoming_request(&receiver.id, "inbox-after-ackall", 1),
        Err(RequestError::Attention(
            AttentionRejection::RevisionConflict {
                current: 2,
                expected: 1
            }
        ))
    ));
}

#[test]
fn same_identity_self_delivery_keeps_request_and_response_roles_separate() {
    let mut fixture = Fixture::new();
    let identity = fixture.identity_id.clone();
    let route = queue_inbox(&mut fixture, "inbox-self", &identity, &identity);
    service(&mut fixture)
        .submit_response(SubmitResponse {
            request_id: "inbox-self".into(),
            proof: ResponseProof::Compact(tmt_core::request::correlation::response_token(
                "inbox-self",
                "inbox-self-attempt",
                &route,
            )),
            body: "self response".into(),
        })
        .unwrap();

    let page = service(&mut fixture)
        .list_incoming(&identity, None, None, None)
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(
        page.items[0].kind,
        tmt_core::request::attention::IncomingKind::Request
    );
    assert_eq!(page.items[0].exchange.revision, 2);
    assert_eq!(
        page.items[1].kind,
        tmt_core::request::attention::IncomingKind::Response
    );
    assert_eq!(page.items[1].exchange.revision, 3);

    service(&mut fixture)
        .acknowledge_incoming_request(&identity, "inbox-self", 2)
        .unwrap();
    let remaining = service(&mut fixture)
        .list_incoming(&identity, None, None, None)
        .unwrap();
    assert_eq!(remaining.items.len(), 1);
    assert_eq!(
        remaining.items[0].kind,
        tmt_core::request::attention::IncomingKind::Response
    );
    service(&mut fixture)
        .acknowledge_exchange(&identity, "inbox-self", 3)
        .unwrap();
    assert!(
        service(&mut fixture)
            .list_incoming(&identity, None, None, None)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn retired_recipient_attention_never_moves_to_a_same_name_replacement() {
    let mut fixture = Fixture::new();
    let sender = fixture.identity_id.clone();
    let original = create_or_resolve(&mut fixture.storage, "Replaceable Inbox", Lifetime::Saved)
        .unwrap()
        .identity;
    queue_inbox(&mut fixture, "inbox-retired", &sender, &original.id);
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms = ? WHERE id = ?",
            params![NOW_MS as i64, original.id],
        )
        .unwrap();
    let replacement = create_or_resolve(&mut fixture.storage, "Replaceable Inbox", Lifetime::Saved)
        .unwrap()
        .identity;
    assert_ne!(replacement.id, original.id);
    assert!(
        service(&mut fixture)
            .list_incoming(&replacement.id, None, None, None)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        service(&mut fixture)
            .list_incoming(&original.id, None, None, None)
            .unwrap()
            .items[0]
            .exchange
            .request_id,
        "inbox-retired"
    );
}
