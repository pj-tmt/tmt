use super::support::{
    DAY_MS, Fixture, NOW_MS, count_rows, endpoint, preamble_count, prepare_input, service,
};
use crate::storage::{Storage, StorageError};
use rusqlite::OptionalExtension;
use std::path::Path;
use tmt_core::request::{
    FinalResponse, Originator, PreambleReservation, PrepareRequest, PreparedRequest, RequestError,
    RequestRoute, RequestService, ResponseProof, ResponseRejection, SubmitResponse,
    correlation::response_token,
};

use crate::storage::test_support::{Operation, concurrent_pair};

/// Run two operations in a chosen order on two connections that were opened
/// before either operation started. These cases model the old worker tests,
/// which deliberately released one independent process before releasing the
/// other rather than asserting on an unstructured race.
fn ordered_pair<T>(database: &Path, operations: [Operation<T>; 2], first: usize) -> [T; 2] {
    assert!(first < 2, "ordered pair index must be 0 or 1");
    let [mut first_storage, mut second_storage] = [
        Storage::open(database).unwrap(),
        Storage::open(database).unwrap(),
    ];
    let [operation_zero, operation_one] = operations;
    let (first_operation, second_operation, first_index) = if first == 0 {
        (operation_zero, operation_one, 0)
    } else {
        (operation_one, operation_zero, 1)
    };
    let first_result = first_operation(&mut first_storage);
    first_storage.close().expect("close first ordered storage");
    let second_result = second_operation(&mut second_storage);
    second_storage
        .close()
        .expect("close second ordered storage");
    if first_index == 0 {
        [first_result, second_result]
    } else {
        [second_result, first_result]
    }
}

#[derive(Debug, Eq, PartialEq)]
struct RequestState {
    status: String,
    wait_active: i64,
    cadence_reserved: i64,
    response_submitted_at_ms: Option<i64>,
    wait_released_at_ms: Option<i64>,
    attention_revision: i64,
    attention_acknowledged_revision: i64,
    latest_attention_revision: i64,
}

fn request_state(database: &Path, request_id: &str) -> RequestState {
    let oracle =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    oracle
        .query_row(
            "SELECT status, wait_active, cadence_reserved, response_submitted_at_ms,
                    wait_released_at_ms, attention_revision,
                    attention_acknowledged_revision,
                    COALESCE((SELECT latest_revision
                              FROM request_attention_identities
                              WHERE identity_id = a.originator_identity_id), 0)
             FROM request_attempts a WHERE request_id = ?",
            [request_id],
            |row| {
                Ok(RequestState {
                    status: row.get(0)?,
                    wait_active: row.get(1)?,
                    cadence_reserved: row.get(2)?,
                    response_submitted_at_ms: row.get(3)?,
                    wait_released_at_ms: row.get(4)?,
                    attention_revision: row.get(5)?,
                    attention_acknowledged_revision: row.get(6)?,
                    latest_attention_revision: row.get(7)?,
                })
            },
        )
        .unwrap()
}

fn response_state(database: &Path, request_id: &str) -> Option<(String, i64, i64)> {
    let oracle =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    oracle
        .query_row(
            "SELECT body, body_bytes, submitted_at_ms
             FROM request_responses WHERE request_id = ?",
            [request_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .unwrap()
}

#[test]
fn acknowledgement_racing_a_final_never_silently_consumes_a_later_revision() {
    for bulk in [false, true] {
        let mut fixture = Fixture::new();
        let identity = fixture.identity_id.clone();
        let target = endpoint("%81", 181);
        let input = prepare_input(
            &fixture,
            "attention-race",
            target.clone(),
            false,
            NOW_MS + 3_600_001,
            Originator::Explicit(identity.clone()),
            false,
        );
        service(&mut fixture)
            .prepare(input, "attention-race-attempt".into(), 7)
            .unwrap();
        service(&mut fixture)
            .begin_send("attention-race-attempt")
            .unwrap();
        service(&mut fixture)
            .settle(
                "attention-race-attempt",
                tmt_core::request::Settlement::Sent,
            )
            .unwrap();
        let acknowledge: Operation<Result<u64, RequestError<StorageError>>> =
            Box::new(move |storage| {
                let mut service = RequestService::new(storage, || NOW_MS);
                if bulk {
                    service.acknowledge_all_exchanges(&identity)
                } else {
                    service
                        .acknowledge_exchange(&identity, "attention-race", 1)
                        .map(|ack| u64::from(ack.changed))
                }
            });
        let submit: Operation<Result<u64, RequestError<StorageError>>> = Box::new(move |storage| {
            RequestService::new(storage, || NOW_MS)
                .submit_response(SubmitResponse {
                    request_id: "attention-race".into(),
                    proof: ResponseProof::Recorded {
                        attempt_id: "attention-race-attempt".into(),
                        endpoint: target,
                    },
                    body: "exact racing final\r\n".into(),
                })
                .map(|_| 0)
        });
        let [ack, submitted] = concurrent_pair(&fixture.database, [acknowledge, submit]);
        submitted.unwrap();
        let oracle = rusqlite::Connection::open_with_flags(
            &fixture.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let (latest, through, revision, individual, body): (i64, i64, i64, i64, String) = oracle.query_row(
            "SELECT s.latest_revision, s.acknowledged_through, a.attention_revision,
                    a.attention_acknowledged_revision, r.body
             FROM request_attempts a JOIN request_attention_identities s ON s.identity_id = a.originator_identity_id
             JOIN request_responses r USING(request_id) WHERE a.request_id = 'attention-race'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).unwrap();
        assert_eq!((latest, revision), (2, 2));
        assert_eq!(body, "exact racing final\r\n");
        if bulk {
            let cutoff = ack.unwrap();
            assert!([1, 2].contains(&cutoff));
            assert_eq!((through, individual), (i64::try_from(cutoff).unwrap(), 0));
        } else {
            assert_eq!(through, 0);
            match ack {
                Ok(changed) => {
                    assert_eq!(changed, 1);
                    assert_eq!(individual, 1);
                }
                Err(RequestError::Attention(
                    tmt_core::request::attention::AttentionRejection::RevisionConflict {
                        current: 2,
                        expected: 1,
                    },
                )) => assert_eq!(individual, 0),
                unexpected => panic!("unexpected concurrent acknowledgement: {unexpected:?}"),
            }
        }
        let identity = fixture.identity_id.clone();
        let pending = service(&mut fixture)
            .list_exchanges(&identity, None, None)
            .unwrap();
        assert_eq!(pending.items.len(), usize::from(through < 2));
        if let Some(item) = pending.items.first() {
            assert_eq!(item.revision, 2);
            assert!(!item.acknowledged);
        }
    }
}

#[test]
fn concurrent_prepare_connections_serialize_cadence_and_preserve_both_attempts() {
    let fixture = Fixture::new();
    let operations: [Operation<Result<PreparedRequest, RequestError<StorageError>>>; 2] =
        ["a", "b"].map(|suffix| {
            let identity_id = fixture.identity_id.clone();
            Box::new(move |storage: &mut Storage| {
                let input = PrepareRequest {
                    origin: tmt_core::request::RequestOrigin::Person,
                    room_id: None,
                    kind: tmt_core::request::RequestKind::Request,
                    request_id: format!("request-{suffix}"),
                    message: format!("prompt-{suffix}"),
                    route: RequestRoute::Pane(endpoint("%60", 160)),
                    wait: true,
                    expires_at_ms: NOW_MS + 3_600_001,
                    originator: Originator::Unknown,
                    recipient_identity_id: Some(identity_id.clone()),
                    preamble: Some(PreambleReservation {
                        identity_id,
                        every: 3,
                    }),
                };
                RequestService::new(storage, || NOW_MS).prepare(
                    input,
                    format!("attempt-{suffix}"),
                    7,
                )
            }) as Operation<_>
        });
    let prepared = concurrent_pair(&fixture.database, operations).map(Result::unwrap);
    assert_eq!(prepared.iter().filter(|p| p.inject_preamble).count(), 1);
    assert_eq!(
        prepared
            .iter()
            .filter(|p| p.previous_request_id.is_some())
            .count(),
        1
    );
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 2);
    let oracle = rusqlite::Connection::open_with_flags(
        &fixture.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let rows: Vec<(String, i64, i64, String)> = oracle.prepare(
        "SELECT attempt_id, inject_preamble, cadence_reserved, status FROM request_attempts ORDER BY attempt_id",
    ).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(
        rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        ["attempt-a", "attempt-b"]
    );
    assert_eq!(rows.iter().filter(|r| r.1 == 1).count(), 1);
    assert!(rows.iter().all(|r| r.2 == 1 && r.3 == "prepared"));
}

#[test]
fn identical_and_conflicting_final_writers_keep_one_body_marker_and_revision() {
    for bodies in [["same", "same"], ["first", "second"]] {
        let mut fixture = Fixture::new();
        let input = prepare_input(
            &fixture,
            "final-race",
            endpoint("%61", 161),
            false,
            NOW_MS + 1,
            Originator::Explicit(fixture.identity_id.clone()),
            false,
        );
        service(&mut fixture)
            .prepare(input, "final-race-attempt".into(), 7)
            .unwrap();
        service(&mut fixture)
            .begin_send("final-race-attempt")
            .unwrap();
        let operations: [Operation<Result<FinalResponse, RequestError<StorageError>>>; 2] = [0, 1]
            .map(|index| {
                Box::new(move |storage: &mut Storage| {
                    RequestService::new(storage, || NOW_MS + 10 + index as u64).submit_response(
                        SubmitResponse {
                            request_id: "final-race".into(),
                            proof: if index == 0 {
                                ResponseProof::Recorded {
                                    attempt_id: "final-race-attempt".into(),
                                    endpoint: endpoint("%61", 161),
                                }
                            } else {
                                ResponseProof::Compact(response_token(
                                    "final-race",
                                    "final-race-attempt",
                                    &RequestRoute::Pane(endpoint("%61", 161)),
                                ))
                            },
                            body: bodies[index].into(),
                        },
                    )
                }) as Operation<_>
            });
        let results = concurrent_pair(&fixture.database, operations);
        let winners: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        if bodies[0] == bodies[1] {
            assert_eq!(winners.len(), 2);
            assert_eq!(winners[0], winners[1]);
        } else {
            assert_eq!(winners.len(), 1);
            assert_eq!(
                results
                    .iter()
                    .filter(|r| matches!(
                        r,
                        Err(RequestError::Response(ResponseRejection::Conflict))
                    ))
                    .count(),
                1
            );
        }
        assert_eq!(count_rows(&fixture.database, "request_responses"), 1);
        let oracle = rusqlite::Connection::open_with_flags(
            &fixture.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let stored: (String, i64, i64, i64, i64) = oracle
            .query_row(
                "SELECT r.body, r.submitted_at_ms, a.response_submitted_at_ms,
                    a.attention_revision, s.latest_revision
             FROM request_responses r JOIN request_attempts a USING(request_id)
             JOIN request_attention_identities s ON s.identity_id = a.originator_identity_id",
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
        assert_eq!(
            stored,
            (
                winners[0].body.clone(),
                winners[0].submitted_at_ms as i64,
                winners[0].submitted_at_ms as i64,
                2,
                2
            )
        );
    }
}

#[test]
fn finalization_and_failure_orders_preserve_or_reject_exactly() {
    for final_first in [true, false] {
        let mut fixture = Fixture::new();
        let request_id = if final_first {
            "request-final-first"
        } else {
            "request-failure-first"
        };
        let attempt_id = if final_first {
            "attempt-final-first"
        } else {
            "attempt-failure-first"
        };
        let target = endpoint("%63", 163);
        let input = prepare_input(
            &fixture,
            request_id,
            target.clone(),
            true,
            NOW_MS + 3_600_001,
            Originator::Explicit(fixture.identity_id.clone()),
            true,
        );
        service(&mut fixture)
            .prepare(input, attempt_id.into(), 7)
            .unwrap();
        service(&mut fixture).begin_send(attempt_id).unwrap();

        let race_now = NOW_MS + 10;
        let final_request_id = request_id.to_owned();
        let final_attempt_id = attempt_id.to_owned();
        let final_target = target.clone();
        let final_submission: Operation<Result<(), RequestError<StorageError>>> =
            Box::new(move |storage| {
                RequestService::new(storage, move || race_now)
                    .submit_response(SubmitResponse {
                        request_id: final_request_id,
                        proof: ResponseProof::Recorded {
                            attempt_id: final_attempt_id,
                            endpoint: final_target,
                        },
                        body: "final body".into(),
                    })
                    .map(|_| ())
            });
        let failure_attempt_id = attempt_id.to_owned();
        let definite_failure: Operation<Result<(), RequestError<StorageError>>> =
            Box::new(move |storage| {
                RequestService::new(storage, move || race_now).settle(
                    &failure_attempt_id,
                    tmt_core::request::Settlement::DefinitelyFailed,
                )
            });
        let [submitted, settled] = if final_first {
            ordered_pair(&fixture.database, [final_submission, definite_failure], 0)
        } else {
            ordered_pair(&fixture.database, [final_submission, definite_failure], 1)
        };

        if final_first {
            assert!(
                submitted.is_ok(),
                "finalization should win when ordered first"
            );
            assert!(
                settled.is_ok(),
                "failure after finalization should settle uncertain"
            );
            assert_eq!(
                request_state(&fixture.database, request_id),
                RequestState {
                    status: "uncertain".into(),
                    wait_active: 1,
                    cadence_reserved: 1,
                    response_submitted_at_ms: Some(race_now as i64),
                    wait_released_at_ms: None,
                    attention_revision: 2,
                    attention_acknowledged_revision: 0,
                    latest_attention_revision: 2,
                }
            );
            assert_eq!(
                response_state(&fixture.database, request_id),
                Some(("final body".into(), 10, race_now as i64))
            );
            assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 1);
        } else {
            assert!(settled.is_ok(), "definite failure should settle first");
            assert!(matches!(
                submitted,
                Err(RequestError::Response(ResponseRejection::StateInvalid))
            ));
            assert_eq!(
                request_state(&fixture.database, request_id),
                RequestState {
                    status: "definitely_failed".into(),
                    wait_active: 1,
                    cadence_reserved: 0,
                    response_submitted_at_ms: None,
                    wait_released_at_ms: None,
                    attention_revision: 1,
                    attention_acknowledged_revision: 0,
                    latest_attention_revision: 1,
                }
            );
            assert_eq!(response_state(&fixture.database, request_id), None);
            assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
            assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 0);
        }
    }
}

#[test]
fn equal_expiry_finalization_is_rejected_before_or_after_cleanup() {
    for cleanup_first in [true, false] {
        let mut fixture = Fixture::new();
        let request_id = if cleanup_first {
            "request-equal-expiry-cleanup-first"
        } else {
            "request-equal-expiry-final-first"
        };
        let attempt_id = if cleanup_first {
            "attempt-equal-expiry-cleanup-first"
        } else {
            "attempt-equal-expiry-final-first"
        };
        let target = endpoint("%64", 164);
        let expires_at = NOW_MS + 3_600_000;
        let acceptance_expiry = NOW_MS + 7 * DAY_MS;
        let input = prepare_input(
            &fixture,
            request_id,
            target.clone(),
            true,
            expires_at,
            Originator::Explicit(fixture.identity_id.clone()),
            true,
        );
        service(&mut fixture)
            .prepare(input, attempt_id.into(), 7)
            .unwrap();
        service(&mut fixture).begin_send(attempt_id).unwrap();

        let cleanup: Operation<Result<(), RequestError<StorageError>>> =
            Box::new(move |storage| RequestService::new(storage, || acceptance_expiry).cleanup());
        let final_request_id = request_id.to_owned();
        let final_attempt_id = attempt_id.to_owned();
        let final_target = target.clone();
        let final_submission: Operation<Result<(), RequestError<StorageError>>> =
            Box::new(move |storage| {
                RequestService::new(storage, || acceptance_expiry)
                    .submit_response(SubmitResponse {
                        request_id: final_request_id,
                        proof: ResponseProof::Recorded {
                            attempt_id: final_attempt_id,
                            endpoint: final_target,
                        },
                        body: "expired body".into(),
                    })
                    .map(|_| ())
            });
        let [cleanup_result, submission_result] = ordered_pair(
            &fixture.database,
            [cleanup, final_submission],
            usize::from(!cleanup_first),
        );
        assert!(cleanup_result.is_ok());
        assert!(matches!(
            submission_result,
            Err(RequestError::Response(ResponseRejection::Expired))
        ));
        assert_eq!(
            request_state(&fixture.database, request_id),
            RequestState {
                status: "uncertain".into(),
                wait_active: 0,
                cadence_reserved: 1,
                response_submitted_at_ms: None,
                wait_released_at_ms: Some(acceptance_expiry as i64),
                attention_revision: 1,
                attention_acknowledged_revision: 0,
                latest_attention_revision: 1,
            }
        );
        assert_eq!(response_state(&fixture.database, request_id), None);
        assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
        assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 1);
    }
}

#[test]
fn cleanup_and_late_submission_serialize_without_refund_or_lost_final() {
    let mut fixture = Fixture::new();
    let input = prepare_input(
        &fixture,
        "cleanup-race",
        endpoint("%62", 162),
        true,
        NOW_MS + 1,
        Originator::Unknown,
        true,
    );
    service(&mut fixture)
        .prepare(input, "cleanup-race-attempt".into(), 1)
        .unwrap();
    service(&mut fixture)
        .begin_send("cleanup-race-attempt")
        .unwrap();
    let now = NOW_MS + 6 * DAY_MS + 10;
    let results = concurrent_pair(
        &fixture.database,
        [
            Box::new(move |storage| {
                RequestService::new(storage, || now)
                    .cleanup()
                    .map(|()| None)
            }),
            Box::new(move |storage| {
                RequestService::new(storage, || now)
                    .submit_response(SubmitResponse {
                        request_id: "cleanup-race".into(),
                        proof: ResponseProof::Compact(response_token(
                            "cleanup-race",
                            "cleanup-race-attempt",
                            &RequestRoute::Pane(endpoint("%62", 162)),
                        )),
                        body: "late final".into(),
                    })
                    .map(Some)
            }),
        ],
    )
    .map(Result::unwrap);
    assert!(results[0].is_none());
    let final_response = results[1].as_ref().unwrap();
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 1);
    let oracle = rusqlite::Connection::open_with_flags(
        &fixture.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let state: (String, i64, Option<String>, i64) = oracle.query_row(
        "SELECT status, wait_active, message_text, response_submitted_at_ms FROM request_attempts WHERE request_id = 'cleanup-race'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap();
    assert_eq!(state, ("uncertain".into(), 0, None, now as i64));
    drop(oracle);
    fixture.set_now(NOW_MS + 7 * DAY_MS);
    assert_eq!(
        service(&mut fixture).get_response("cleanup-race").unwrap(),
        tmt_core::request::ResponseLookup::Available(Box::new(final_response.clone()))
    );
}
